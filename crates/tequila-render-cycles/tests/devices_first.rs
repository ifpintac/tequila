//! Asking which devices there are, before any session is made, leaves the
//! graphics card usable. A process of its own, because what this tests is
//! the first thing a process does with Cycles: Cycles decides once whether
//! CUDA can be used - by whether it finds GPU kernels - and the device list
//! asked for before the kernel folder was set decided that it could not,
//! for good. The Render window lists devices as it opens, and a render on
//! the graphics card then failed with "no CUDA device is available".
//!
//! Skipped where Cycles is not installed, and unless
//! `TEQUILA_TEST_CYCLES_GPU` says there is a card to test with.

#[test]
fn listing_devices_first_leaves_the_card_usable() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_none() {
        eprintln!("skipped: TEQUILA_TEST_CYCLES_GPU is not set");
        return;
    }
    let devices = tequila_render_cycles::devices().expect("lists devices");
    assert!(
        devices.iter().any(|(kind, _, _)| kind == "CUDA" || kind == "OPTIX"),
        "listed first, the devices left out the graphics card: {devices:?}"
    );
    tequila_render_cycles::Session::new("CUDA", 0, None).expect("a session on the graphics card");
}
