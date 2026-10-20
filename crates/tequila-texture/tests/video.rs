//! A video's frames, through the system's own decoder, on the clip
//! `tests/data/make_video.py` writes: three seconds at one frame a second,
//! red, green and blue. Windows and macOS only.

#![cfg(any(windows, target_os = "macos"))]

use tequila_texture::video;

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/rgb.mp4");

/// The colour in the middle of a frame.
fn middle(frame: &video::Frame) -> [i32; 3] {
    let i = ((frame.height / 2) * frame.width + frame.width / 2) * 4;
    [frame.rgba[i] as i32, frame.rgba[i + 1] as i32, frame.rgba[i + 2] as i32]
}

fn near(got: [i32; 3], want: [i32; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 24)
}

#[test]
fn a_video_says_how_long_it_runs() {
    let seconds = video::duration(CLIP).expect("the clip has a length");
    assert!((seconds - 3.0).abs() < 0.1, "{seconds} s");
}

/// The frame showing at a time is the last to start at or before it: half
/// way through the second second is still the green one.
#[test]
fn the_frame_at_a_time_is_the_one_showing_then() {
    for (at, want) in [(0.0, [255, 0, 0]), (1.0, [0, 255, 0]), (1.5, [0, 255, 0]), (2.2, [0, 0, 255])] {
        let frame = video::frame(CLIP, at).unwrap_or_else(|e| panic!("no frame at {at} s: {e}"));
        assert_eq!((frame.width, frame.height), (64, 48));
        assert!(near(middle(&frame), want), "at {at} s: {:?}, wanted {want:?}", middle(&frame));
    }
}

#[test]
fn a_file_that_is_not_a_video_is_refused() {
    let not = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/make_video.py");
    assert!(video::frame(not, 0.0).is_err());
    assert!(video::is_video("clip.MOV") && !video::is_video("label.png"));
}
