//! Print speaker lines for one recording, or write one file per card recording.
//!
//! A path that exists is read from disk. Any other argument is an exact file
//! name on the recorder. `--all` writes every recording on the card.
//! Pass `--asr-model <dir>` to add words.

use std::fs;
use std::path::{Path, PathBuf};

use highdock_rs::{
    SpokenLine, Transcript, TranscriptBook, TranscriptEngine, decode_recording, separate_speakers,
    transcribe,
};

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    if args.all {
        let Some(model) = args.asr_model else {
            eprintln!("usage: transcribe --all --asr-model <dir> --out <dir>");
            std::process::exit(2);
        };
        let Some(out) = args.out else {
            eprintln!("usage: transcribe --all --asr-model <dir> --out <dir>");
            std::process::exit(2);
        };
        write_card(&model, &out).await?;
        return Ok(());
    }
    if let Some(dir) = &args.from_lines {
        let Some(out) = args.out else {
            eprintln!("usage: transcribe --from-lines <dir> --out <dir>");
            std::process::exit(2);
        };
        export_lines(dir, &out)?;
        return Ok(());
    }
    let Some(input) = args.input else {
        eprintln!("usage: transcribe <recording> [--asr-model <dir>]");
        std::process::exit(2);
    };
    let bytes = if input.exists() {
        fs::read(&input)?
    } else {
        let name = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.is_empty() {
            eprintln!("usage: transcribe <recording> [--asr-model <dir>]");
            std::process::exit(2);
        }
        pull(name).await?
    };
    let speech = decode_recording(&bytes)?;
    eprintln!("{} samples at 16000 Hz", speech.samples.len());
    let lines = match args.asr_model {
        Some(dir) => transcribe(&speech, &dir)?,
        None => {
            eprintln!("no --asr-model, so the lines have no words");
            separate_speakers(&speech)?
        }
    };
    print_lines(&lines);
    Ok(())
}

struct Args {
    input: Option<PathBuf>,
    asr_model: Option<PathBuf>,
    all: bool,
    out: Option<PathBuf>,
    from_lines: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let mut input = None;
        let mut asr_model = None;
        let mut out = None;
        let mut all = false;
        let mut from_lines = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--asr-model" {
                let Some(dir) = args.next() else {
                    eprintln!("--asr-model needs a directory");
                    std::process::exit(2);
                };
                asr_model = Some(PathBuf::from(dir));
            } else if arg == "--out" {
                let Some(dir) = args.next() else {
                    eprintln!("--out needs a directory");
                    std::process::exit(2);
                };
                out = Some(PathBuf::from(dir));
            } else if arg == "--all" {
                all = true;
            } else if arg == "--from-lines" {
                let Some(dir) = args.next() else {
                    eprintln!("--from-lines needs a directory");
                    std::process::exit(2);
                };
                from_lines = Some(PathBuf::from(dir));
            } else if input.is_none() {
                input = Some(PathBuf::from(arg));
            } else {
                eprintln!("usage: transcribe <recording> [--asr-model <dir>]");
                eprintln!("       transcribe --all --asr-model <dir> --out <dir>");
                eprintln!("       transcribe --from-lines <dir> --out <dir>");
                std::process::exit(2);
            }
        }
        if !all && from_lines.is_none() && input.is_none() {
            eprintln!("usage: transcribe <recording> [--asr-model <dir>]");
            eprintln!("       transcribe --all --asr-model <dir> --out <dir>");
            eprintln!("       transcribe --from-lines <dir> --out <dir>");
            std::process::exit(2);
        }
        Ok(Self {
            input,
            asr_model,
            all,
            out,
            from_lines,
        })
    }
}

async fn write_card(model: &Path, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(out)?;
    let mut engine = TranscriptEngine::open(model)?;
    let mut device = highdock_rs::Device::open_first().await?;
    let recordings = device.list_recordings().await?;
    let mut book = TranscriptBook::new();
    for recording in &recordings {
        if recording.bytes == 0 {
            eprintln!("skip {} (0 bytes)", recording.name);
            continue;
        }
        eprintln!("download {}", recording.name);
        let mut bytes = Vec::with_capacity(recording.bytes as usize);
        device
            .download(recording, |chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })
            .await?;
        let path = out.join(transcript_name(&recording.name));
        let speech = match decode_recording(&bytes) {
            Ok(speech) => speech,
            Err(err) => {
                eprintln!("{err}");
                fs::write(&path, format!("{err}\n"))?;
                continue;
            }
        };
        let lines = match engine.transcribe(&speech) {
            Ok(lines) => lines,
            Err(err) => {
                eprintln!("{err}");
                fs::write(&path, format!("{err}\n"))?;
                continue;
            }
        };
        fs::write(&path, render(&lines))?;
        book.push(Transcript::from_lines(recording.name.clone(), &lines));
        eprintln!("wrote {}", path.display());
    }
    export_book(&book, out)?;
    Ok(())
}

fn export_lines(dir: &Path, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            names.push(path);
        }
    }
    names.sort();
    let mut book = TranscriptBook::new();
    for path in names {
        let text = fs::read_to_string(&path)?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("recording");
        let lines = parse_lines(&text)?;
        book.push(Transcript::from_lines(format!("{stem}.hda"), &lines));
    }
    export_book(&book, out)?;
    Ok(())
}

fn export_book(book: &TranscriptBook, out: &Path) -> Result<(), highdock_rs::Error> {
    book.write_each(out)?;
    let long = out.join("transcripts.json");
    book.write_json(&long)?;
    eprintln!("wrote {}", long.display());
    Ok(())
}

fn parse_lines(text: &str) -> Result<Vec<SpokenLine>, Box<dyn std::error::Error>> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim();
        if raw.is_empty() || raw == "0 speaker turns" {
            continue;
        }
        let mut parts = raw.splitn(3, ' ');
        let speaker = parts.next().unwrap_or("");
        let times = parts.next().unwrap_or("");
        let words = parts.next().unwrap_or("");
        let Some((start, end)) = times.split_once('-') else {
            return Err(format!("a transcript line has no time range: {raw}").into());
        };
        lines.push(SpokenLine {
            speaker: speaker.to_string(),
            start_secs: start.parse()?,
            end_secs: end.parse()?,
            text: words.to_string(),
        });
    }
    Ok(lines)
}

fn transcript_name(recording: &str) -> String {
    let stem = Path::new(recording)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(recording);
    format!("{stem}.txt")
}

fn render(lines: &[SpokenLine]) -> String {
    let mut text = String::new();
    if lines.is_empty() {
        text.push_str("0 speaker turns\n");
        return text;
    }
    for line in lines {
        if line.text.is_empty() {
            text.push_str(&format!(
                "{} {:.2}-{:.2}\n",
                line.speaker, line.start_secs, line.end_secs
            ));
        } else {
            text.push_str(&format!(
                "{} {:.2}-{:.2} {}\n",
                line.speaker, line.start_secs, line.end_secs, line.text
            ));
        }
    }
    text
}

async fn pull(name: &str) -> Result<Vec<u8>, highdock_rs::Error> {
    let mut device = highdock_rs::Device::open_first().await?;
    let recordings = device.list_recordings().await?;
    let Some(index) = recordings.iter().position(|item| item.name == name) else {
        eprintln!("{name} is not a local file and it is not on the card");
        for item in &recordings {
            eprintln!("{}", item.name);
        }
        std::process::exit(2);
    };
    let recording = &recordings[index];
    let mut bytes = Vec::with_capacity(recording.bytes as usize);
    device
        .download(recording, |chunk| {
            bytes.extend_from_slice(chunk);
            Ok(())
        })
        .await?;
    Ok(bytes)
}

fn print_lines(lines: &[SpokenLine]) {
    if lines.is_empty() {
        println!("0 speaker turns");
        return;
    }
    for line in lines {
        if line.text.is_empty() {
            println!(
                "{} {:.2}-{:.2}",
                line.speaker, line.start_secs, line.end_secs
            );
        } else {
            println!(
                "{} {:.2}-{:.2} {}",
                line.speaker, line.start_secs, line.end_secs, line.text
            );
        }
    }
}
