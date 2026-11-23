//! Set the recorder clock.
//!
//! Arguments: year month day hour minute second.

#[path = "common/mod.rs"]
mod common;

use highdock_rs::WallTime;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut args = std::env::args().skip(1);
    let year = number(&mut args, "year");
    let month = number(&mut args, "month");
    let day = number(&mut args, "day");
    let hour = number(&mut args, "hour");
    let minute = number(&mut args, "minute");
    let second = number(&mut args, "second");
    let time = WallTime::new(
        year,
        month as u8,
        day as u8,
        hour as u8,
        minute as u8,
        second as u8,
    )?;

    let mut device = common::open().await?;
    device.set_time(time).await?;
    println!(
        "clock set to {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    );
    Ok(())
}

fn number(args: &mut impl Iterator<Item = String>, label: &str) -> u16 {
    let Some(text) = args.next() else {
        eprintln!("usage: set_time <year> <month> <day> <hour> <minute> <second>");
        std::process::exit(2);
    };
    let Ok(value) = text.parse::<u16>() else {
        eprintln!("{label} is not a number");
        std::process::exit(2);
    };
    value
}
