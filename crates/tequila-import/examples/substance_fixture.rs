//! Write the synthetic Substance material the tests use — a `.sbsar` and an
//! `.sbs` of a brick wall, written from the published format, not by
//! Adobe's tools — into a folder, for trying Tequila by hand.
//!
//!     cargo run --release -p tequila-import --example substance_fixture -- <folder>

fn main() {
    let folder = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&folder).expect("the folder");
    let sbsar = folder.join("bricks_tequila.sbsar");
    std::fs::write(&sbsar, tequila_import::substance::fixture::bricks_sbsar()).expect("writes");
    let sbs = folder.join("bricks_tequila.sbs");
    std::fs::write(&sbs, tequila_import::substance::fixture::bricks_sbs()).expect("writes");
    println!("{}\n{}", sbsar.display(), sbs.display());
}
