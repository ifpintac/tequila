//! Opt-in capture of the app's own framebuffer for visual QA.
//! Set TEQUILA_UI_CAPTURE to a PNG path; normal launches do nothing here.
use eframe::egui;

pub struct UiCapture {
    path: Option<std::path::PathBuf>,
    started: std::time::Instant,
    requested: bool,
}

impl Default for UiCapture {
    fn default() -> Self {
        Self {
            path: std::env::var_os("TEQUILA_UI_CAPTURE").map(Into::into),
            started: std::time::Instant::now(),
            requested: false,
        }
    }
}

impl UiCapture {
    pub fn update(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.path else { return };
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = image {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let file = std::fs::File::create(path)?;
                let mut encoder =
                    png::Encoder::new(file, image.width() as u32, image.height() as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header()?;
                let bytes: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                writer.write_image_data(&bytes)?;
                writer.finish()?;
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("UI capture failed: {error}");
            }
            self.path = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !self.requested && self.started.elapsed().as_secs() >= 5 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
    }
}
