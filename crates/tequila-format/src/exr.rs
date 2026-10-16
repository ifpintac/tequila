//! Multi-layer OpenEXR output (`docs/01-RENDERER-CORE.md` §12).
//!
//! M4's exit criterion ends in a compositor: "open the EXR in a compositor and
//! confirm Cryptomatte mattes select correctly and that Beauty = sum of its
//! component passes". Both halves of that are about a file being *read by
//! something else*, which is why this module is built on the `exr` crate — a
//! complete implementation of the format that this project already depends on
//! for reading — rather than on a writer of our own.
//!
//! That is a different call from the one made for `.tequila`, JSON, ZIP and
//! the image formats, and the difference is worth stating. Those are formats
//! this project *defines* or uses a small, fully-specified corner of. OpenEXR
//! is neither: it is a large format whose value is entirely in other people's
//! tools opening it, and an implementation that is 95% right produces files
//! that open and are subtly wrong — which is the one outcome the exit criterion
//! exists to rule out.
//!
//! # Layout
//!
//! One layer, many channels, named `Pass.R`, `Pass.G`, `Pass.B`. Compositors
//! read that naming as a layer list, and it is what a Cryptomatte requires:
//! `CryptoObject00.R` through `CryptoObject02.A` have to sit beside the beauty
//! channels in the same part, not in parts of their own.

use tequila_core::math::Vec3;
use tequila_core::spectrum::Spectrum;
use std::collections::HashMap;
use std::path::Path;

/// How much precision a channel keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Precision {
    /// 16-bit. Half the size, and about three decimal digits — plenty for a
    /// colour channel that is going to be graded and viewed.
    #[default]
    Half,
    /// 32-bit. What a Cryptomatte, a depth pass and a position pass need: the
    /// first because its identifiers are *bit patterns* and rounding one
    /// changes which object it names, the others because they are distances in
    /// scene units and half a metre of precision at ten metres is not useful.
    Full,
}

/// One channel group to write.
#[derive(Clone, Debug)]
pub struct Layer {
    /// `Beauty`, `Diffuse`, `CryptoObject00`. Empty writes bare `R`/`G`/`B`,
    /// which is what a compositor shows by default.
    pub name: String,
    pub channels: Channels,
    pub precision: Precision,
}

/// The samples of a layer.
#[derive(Clone, Debug)]
pub enum Channels {
    /// Three channels, `R`/`G`/`B`.
    Color(Vec<Spectrum>),
    /// Four channels, `R`/`G`/`B`/`A` — what a Cryptomatte layer is.
    Rgba(Vec<[f32; 4]>),
    /// One channel. Named after the layer when there is one, or `Y` when there
    /// is not, since a bare single channel with no name is not something a
    /// compositor can show.
    Scalar(Vec<f32>),
    /// Three channels holding a direction or a position rather than a colour.
    Vector(Vec<Vec3>),
}

impl Channels {
    fn len(&self) -> usize {
        match self {
            Channels::Color(v) => v.len(),
            Channels::Rgba(v) => v.len(),
            Channels::Scalar(v) => v.len(),
            Channels::Vector(v) => v.len(),
        }
    }
}

/// How hard to work at making the file small.
///
/// Both options are lossless — a render pass is data a compositor does
/// arithmetic on, and a lossy codec would break the very identity the exit
/// criterion checks. What differs is time against size, and at 4K with twelve
/// passes the difference is over a gigabyte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Packing {
    /// ZIP, in blocks of sixteen scan lines. Slower to write, and typically
    /// less than half the size.
    #[default]
    Compressed,
    /// None at all. For a render being handed straight to another process,
    /// where the time spent compressing is time nobody gets back.
    Raw,
}

/// An image on its way to disk.
#[derive(Clone, Debug, Default)]
pub struct ExrImage {
    pub width: usize,
    pub height: usize,
    pub layers: Vec<Layer>,
    /// String attributes in the header. Cryptomatte's metadata lives here.
    pub attributes: Vec<(String, String)>,
    pub packing: Packing,
}

#[derive(Debug)]
pub enum ExrError {
    Empty,
    /// A layer's sample count does not match the image.
    Size { layer: String, expected: usize, found: usize },
    Write(String),
}

impl std::fmt::Display for ExrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExrError::Empty => write!(f, "an EXR with no layers"),
            ExrError::Size {
                layer,
                expected,
                found,
            } => write!(
                f,
                "layer `{layer}` holds {found} samples where the image is {expected} pixels"
            ),
            ExrError::Write(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ExrError {}

impl ExrImage {
    pub fn new(width: usize, height: usize) -> ExrImage {
        ExrImage {
            width,
            height,
            layers: Vec::new(),
            attributes: Vec::new(),
            packing: Packing::default(),
        }
    }

    pub fn push(&mut self, layer: Layer) -> &mut Self {
        self.layers.push(layer);
        self
    }

    pub fn attribute(&mut self, name: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.attributes.push((name.into(), value.into()));
        self
    }

    /// Every channel, fully named, with its samples.
    ///
    /// Split out from the writing so the layout can be tested without a file:
    /// the thing most likely to be wrong here is a channel name, and a name is
    /// checkable without OpenEXR having an opinion about it.
    pub fn channels(&self) -> Result<Vec<(String, Precision, Vec<f32>)>, ExrError> {
        if self.layers.is_empty() {
            return Err(ExrError::Empty);
        }
        let count = self.width * self.height;
        let mut out = Vec::new();
        for layer in &self.layers {
            if layer.channels.len() != count {
                return Err(ExrError::Size {
                    layer: layer.name.clone(),
                    expected: count,
                    found: layer.channels.len(),
                });
            }
            let qualify = |suffix: &str| {
                if layer.name.is_empty() {
                    suffix.to_string()
                } else {
                    format!("{}.{suffix}", layer.name)
                }
            };
            match &layer.channels {
                Channels::Color(samples) => {
                    for (index, suffix) in ["R", "G", "B"].into_iter().enumerate() {
                        out.push((
                            qualify(suffix),
                            layer.precision,
                            samples
                                .iter()
                                .map(|s| s.to_rendering_rgb())
                                .map(|v| [v.x, v.y, v.z][index])
                                .collect(),
                        ));
                    }
                }
                Channels::Vector(samples) => {
                    for (index, suffix) in ["X", "Y", "Z"].into_iter().enumerate() {
                        out.push((
                            qualify(suffix),
                            layer.precision,
                            samples.iter().map(|v| [v.x, v.y, v.z][index]).collect(),
                        ));
                    }
                }
                Channels::Rgba(samples) => {
                    for (index, suffix) in ["R", "G", "B", "A"].into_iter().enumerate() {
                        out.push((
                            qualify(suffix),
                            layer.precision,
                            samples.iter().map(|s| s[index]).collect(),
                        ));
                    }
                }
                Channels::Scalar(samples) => {
                    out.push((
                        if layer.name.is_empty() {
                            "Y".to_string()
                        } else {
                            layer.name.clone()
                        },
                        layer.precision,
                        samples.clone(),
                    ));
                }
            }
        }
        Ok(out)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), ExrError> {
        use exr::prelude::*;

        let channels = self.channels()?;
        let size = (self.width, self.height);

        let list: Vec<AnyChannel<FlatSamples>> = channels
            .into_iter()
            .map(|(name, precision, samples)| {
                let data = match precision {
                    Precision::Half => FlatSamples::F16(
                        samples.iter().copied().map(half::f16::from_f32).collect(),
                    ),
                    Precision::Full => FlatSamples::F32(samples.into()),
                };
                AnyChannel::new(name.as_str(), data)
            })
            .collect();

        let mut attributes = LayerAttributes::named("tequila");
        let mut other: HashMap<Text, AttributeValue> = HashMap::new();
        for (name, value) in &self.attributes {
            if let Some(name) = Text::new_or_none(name.as_str()) {
                other.insert(name, AttributeValue::Text(Text::from(value.as_str())));
            }
        }
        attributes.other = other;

        let encoding = match self.packing {
            Packing::Compressed => Encoding::SMALL_LOSSLESS,
            Packing::Raw => Encoding::UNCOMPRESSED,
        };
        let layer = exr::image::Layer::new(
            size,
            attributes,
            encoding,
            AnyChannels::sort(list.into_iter().collect()),
        );
        exr::image::Image::from_layer(layer)
            .write()
            .to_file(path.as_ref())
            .map_err(|e| ExrError::Write(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> ExrImage {
        let mut out = ExrImage::new(2, 2);
        out.push(Layer {
            name: String::new(),
            channels: Channels::Color(vec![
                Spectrum::splat(0.1),
                Spectrum::splat(0.2),
                Spectrum::splat(0.3),
                Spectrum::splat(0.4),
            ]),
            precision: Precision::Half,
        });
        out.push(Layer {
            name: "Diffuse".into(),
            channels: Channels::Color(vec![Spectrum::splat(0.5); 4]),
            precision: Precision::Half,
        });
        out.push(Layer {
            name: "Depth".into(),
            channels: Channels::Scalar(vec![1.0, 2.0, 3.0, 4.0]),
            precision: Precision::Full,
        });
        out
    }

    /// The channel names *are* the layer structure, as far as a compositor is
    /// concerned. A beauty pass has to be bare `R`/`G`/`B` — that is what a
    /// viewer shows without being asked — and everything else has to be
    /// prefixed, or a dozen passes all claim to be the image.
    #[test]
    fn channel_names_are_what_a_compositor_reads() {
        let channels = image().channels().unwrap();
        let names: Vec<&str> = channels.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"R"), "{names:?}");
        assert!(names.contains(&"Diffuse.R"), "{names:?}");
        // A single-channel layer is named after the layer, not `Layer.Y`:
        // `Depth` is the convention every compositor looks for.
        assert!(names.contains(&"Depth"), "{names:?}");
        assert_eq!(channels.len(), 3 + 3 + 1);
    }

    /// A Cryptomatte layer is four channels, and its identifiers are bit
    /// patterns rather than numbers — rounding one to half precision changes
    /// which object it names, so those channels must be full float.
    #[test]
    fn a_cryptomatte_layer_is_four_full_float_channels() {
        let mut out = ExrImage::new(1, 1);
        out.push(Layer {
            name: "CryptoObject00".into(),
            channels: Channels::Rgba(vec![[1.0, 0.5, 2.0, 0.5]]),
            precision: Precision::Full,
        });
        let channels = out.channels().unwrap();
        assert_eq!(channels.len(), 4);
        for (name, precision, _) in &channels {
            assert!(name.starts_with("CryptoObject00."));
            assert_eq!(*precision, Precision::Full);
        }
        assert_eq!(channels[3].0, "CryptoObject00.A");
    }

    /// A layer that is the wrong size is refused rather than written short. A
    /// truncated EXR opens, shows most of an image, and gives no clue why.
    #[test]
    fn a_mismatched_layer_is_refused() {
        let mut out = ExrImage::new(4, 4);
        out.push(Layer {
            name: "Diffuse".into(),
            channels: Channels::Color(vec![Spectrum::ZERO; 3]),
            precision: Precision::Half,
        });
        let error = out.channels().unwrap_err();
        assert!(matches!(error, ExrError::Size { .. }), "{error}");
        assert!(ExrImage::new(4, 4).channels().is_err());
    }

    /// The round trip that matters: what is written comes back. Read with the
    /// same library a compositor's behaviour is being stood in for by, which is
    /// the closest this can get to the exit criterion without one.
    #[test]
    fn what_is_written_reads_back() {
        let directory = std::env::temp_dir().join("tequila-exr-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("round-trip.exr");

        let mut out = ExrImage::new(4, 3);
        let pixels: Vec<Spectrum> = (0..12)
            .map(|i| tequila_core::spectrum::spectrum(i as f32 * 0.1, 0.5, 1.0))
            .collect();
        out.push(Layer {
            name: String::new(),
            channels: Channels::Color(pixels.clone()),
            precision: Precision::Full,
        });
        out.push(Layer {
            name: "CryptoObject00".into(),
            channels: Channels::Rgba(vec![[1.5, 0.25, -2.5, 0.75]; 12]),
            precision: Precision::Full,
        });
        out.attribute("cryptomatte/abcd123/name", "CryptoObject");
        out.write(&path).expect("the file is written");

        let read = exr::prelude::read_all_flat_layers_from_file(&path).expect("it reads back");
        let layer = &read.layer_data[0];
        assert_eq!(layer.size.0, 4);
        assert_eq!(layer.size.1, 3);

        let names: Vec<String> = layer
            .channel_data
            .list
            .iter()
            .map(|c| c.name.to_string())
            .collect();
        for expected in ["R", "G", "B", "CryptoObject00.R", "CryptoObject00.A"] {
            assert!(names.contains(&expected.to_string()), "{names:?}");
        }

        // The attribute survived, which is what a Cryptomatte's manifest rides
        // in — a file whose channels are right and whose metadata is missing is
        // one a compositor cannot offer a matte list for.
        let attribute = layer
            .attributes
            .other
            .iter()
            .find(|(k, _)| k.to_string() == "cryptomatte/abcd123/name");
        assert!(attribute.is_some(), "{:?}", layer.attributes.other);

        // And a negative identifier came back intact. Cryptomatte ids are bit
        // patterns and about half of them are negative; a writer that clamped
        // at zero would lose half the objects in a scene.
        let channel = layer
            .channel_data
            .list
            .iter()
            .find(|c| c.name.to_string() == "CryptoObject00.B")
            .expect("the third Cryptomatte channel");
        let value = match &channel.sample_data {
            exr::prelude::FlatSamples::F32(v) => v[0],
            other => panic!("expected full float, found {other:?}"),
        };
        assert_eq!(value, -2.5);

        let _ = std::fs::remove_file(&path);
    }
}
