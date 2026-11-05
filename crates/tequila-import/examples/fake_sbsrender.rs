//! A stand-in for Adobe's `sbsrender` and `sbscooker`, for trying
//! Tequila's Substance materials on a computer without Adobe's tools.
//!
//! Copy the built program into a folder twice, as `sbsrender` and
//! `sbscooker` (with `.exe` on Windows), and point Tequila at the folder
//! (the Substance panel's *Set the folder of Adobe's tools*, or
//! `TEQUILA_SUBSTANCE_TOOLS`). It takes the same arguments and writes the
//! same files, but every map it draws is a picture of bricks: it is not
//! Adobe's engine and knows nothing of what a real graph draws.
//!
//!     cargo build --release -p tequila-import --example fake_sbsrender

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(tequila_import::substance::fake::main(&args));
}
