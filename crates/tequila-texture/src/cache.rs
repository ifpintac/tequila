//! A texture cache with a memory budget (`docs/02-MATERIAL-SYSTEM.md` §5.1).
//!
//! §5.1 sets the requirement as a number rather than an adjective: "product
//! scenes with 40 × 4K textures must work on 8 GB machines". Forty 4K RGB
//! textures held as float pixels is 40 × 4096² × 3 × 4 bytes, which is **8 GB**
//! before the MIP pyramids add their third — so the naive approach does not
//! merely strain that machine, it exceeds its entire memory on the textures
//! alone. Something has to evict.
//!
//! What makes eviction safe here is that a texture is *reconstructible*: it came
//! from a file, and it can come from that file again. Dropping one costs a
//! reload, not a wrong image. So the cache holds what fits, evicts what has gone
//! longest unused, and reloads on the next lookup.
//!
//! ## Why least-recently-used, specifically
//!
//! A path tracer's access pattern is close to the best case for LRU. Rays are
//! traced in screen tiles, so the surfaces hit within one tile are a small,
//! spatially coherent set, and their textures are hit over and over before the
//! renderer moves on. A least-recently-used policy keeps exactly that working
//! set and evicts the textures belonging to parts that are off-screen or behind
//! something. A random or first-in policy would evict the texture currently
//! under the ray about as readily as one nothing has touched in a minute.
//!
//! ## What this is not, yet
//!
//! §5.1 also asks for tile-level residency and asynchronous loading. This is
//! whole-texture granularity and synchronous loading: it bounds memory, which is
//! the requirement that decides whether a scene opens at all, but a 4K texture
//! is loaded entire even when one corner is visible, and the first ray to touch
//! an evicted texture waits for the disk. Both are refinements of this
//! structure rather than replacements for it — the eviction accounting is the
//! part that has to be right first.

use crate::texture::Texture;
use std::collections::HashMap;

/// Bytes one texture occupies, pyramid included.
fn footprint(texture: &Texture) -> usize {
    texture.pyramid().texels()
        * texture.pyramid().base().channels
        * std::mem::size_of::<f32>()
}

/// A cache entry and its last-use stamp.
#[derive(Debug)]
struct Entry {
    texture: Texture,
    bytes: usize,
    /// Monotonic counter rather than a clock: a clock can read the same value
    /// twice in a row on a fast machine, and two entries that tie are two
    /// entries the policy cannot order.
    used: u64,
}

/// How the cache has been performing, for the HUD and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Textures that could not be held even alone, because they exceed the
    /// whole budget.
    pub oversized: u64,
}

impl CacheStats {
    /// Fraction of lookups served without a reload. Below about 0.9 in a
    /// steady state, the budget is too small for the scene and the renderer
    /// will be waiting on the disk rather than on arithmetic.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            1.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Textures held under a byte budget, evicting least-recently-used.
#[derive(Debug)]
pub struct TextureCache {
    entries: HashMap<String, Entry>,
    budget: usize,
    used: usize,
    clock: u64,
    stats: CacheStats,
}

impl TextureCache {
    /// A cache holding at most `budget` bytes of texture data.
    pub fn new(budget: usize) -> TextureCache {
        TextureCache {
            entries: HashMap::new(),
            budget,
            used: 0,
            clock: 0,
            stats: CacheStats::default(),
        }
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Fetch a texture, loading it through `load` if it is not resident.
    ///
    /// `load` is called only on a miss, and is allowed to fail — a missing or
    /// corrupt file is not a reason to bring the render down.
    pub fn get_or_load<F, E>(&mut self, key: &str, load: F) -> Result<&Texture, E>
    where
        F: FnOnce() -> Result<Texture, E>,
    {
        self.clock += 1;
        let now = self.clock;

        if let Some(entry) = self.entries.get_mut(key) {
            entry.used = now;
            self.stats.hits += 1;
            return Ok(&self.entries[key].texture);
        }

        self.stats.misses += 1;
        let texture = load()?;
        let bytes = footprint(&texture);

        // A texture larger than the whole budget is still returned — refusing
        // to shade a surface because its texture is big would be a worse
        // failure than exceeding the budget — but it is recorded, because a
        // budget that cannot hold a single texture is a misconfiguration
        // somebody should hear about.
        if bytes > self.budget {
            self.stats.oversized += 1;
            self.evict_to(0);
        } else {
            self.evict_to(self.budget - bytes);
        }

        self.used += bytes;
        self.entries.insert(
            key.to_string(),
            Entry {
                texture,
                bytes,
                used: now,
            },
        );
        Ok(&self.entries[key].texture)
    }

    /// Put a texture in directly, as an importer does when it has already
    /// decoded one.
    pub fn insert(&mut self, key: &str, texture: Texture) {
        self.clock += 1;
        let bytes = footprint(&texture);
        if let Some(previous) = self.entries.remove(key) {
            self.used -= previous.bytes;
        }
        if bytes > self.budget {
            self.stats.oversized += 1;
            self.evict_to(0);
        } else {
            self.evict_to(self.budget - bytes);
        }
        self.used += bytes;
        let used = self.clock;
        self.entries.insert(
            key.to_string(),
            Entry {
                texture,
                bytes,
                used,
            },
        );
    }

    /// Drop entries, least recently used first, until usage is at or below
    /// `target`.
    fn evict_to(&mut self, target: usize) {
        while self.used > target && !self.entries.is_empty() {
            // The oldest stamp wins. Scanning is fine at the scale a scene
            // reaches — tens to low hundreds of textures — and a heap would
            // need re-keying on every hit, which is the common case.
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.used -= entry.bytes;
                self.stats.evictions += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{ColorSpace, Image};
    use crate::projection::Mapping;
    use crate::texture::MapRole;
    use tequila_core::math::Vec3;

    /// A texture of a given side length, so its size is predictable.
    fn texture(size: usize) -> Texture {
        let mut image = Image::constant(size, size, Vec3::splat(0.5));
        image.color_space = ColorSpace::Raw;
        Texture::from_linear(image, MapRole::Data, Mapping::default())
    }

    fn bytes_of(size: usize) -> usize {
        footprint(&texture(size))
    }

    #[derive(Debug)]
    struct Never;

    #[test]
    fn a_hit_does_not_reload() {
        let mut cache = TextureCache::new(bytes_of(16) * 4);
        let mut loads = 0;
        for _ in 0..5 {
            let _ = cache.get_or_load::<_, Never>("a", || {
                loads += 1;
                Ok(texture(16))
            });
        }
        assert_eq!(loads, 1, "a resident texture was reloaded");
        assert_eq!(cache.stats().hits, 4);
        assert_eq!(cache.stats().misses, 1);
    }

    /// The requirement, stated as §5.1 states it: usage never exceeds the
    /// budget, however many textures arrive.
    #[test]
    fn usage_never_exceeds_the_budget() {
        let one = bytes_of(32);
        let mut cache = TextureCache::new(one * 3);

        for i in 0..40 {
            let key = format!("texture{i}");
            let _ = cache.get_or_load::<_, Never>(&key, || Ok(texture(32)));
            assert!(
                cache.used() <= cache.budget(),
                "after {} textures the cache held {} bytes against a budget of {}",
                i + 1,
                cache.used(),
                cache.budget()
            );
        }
        assert_eq!(cache.len(), 3, "the budget should hold exactly three");
        assert!(cache.stats().evictions > 0);
    }

    /// The point of least-recently-used: the texture under the ray survives,
    /// and the one nothing has touched goes.
    #[test]
    fn the_least_recently_used_texture_is_the_one_evicted() {
        let one = bytes_of(16);
        let mut cache = TextureCache::new(one * 2);

        let _ = cache.get_or_load::<_, Never>("old", || Ok(texture(16)));
        let _ = cache.get_or_load::<_, Never>("hot", || Ok(texture(16)));
        // Touch "hot" so "old" is the stale one.
        let _ = cache.get_or_load::<_, Never>("hot", || Ok(texture(16)));

        // A third texture forces an eviction.
        let _ = cache.get_or_load::<_, Never>("new", || Ok(texture(16)));

        assert!(cache.contains("hot"), "the recently used texture was evicted");
        assert!(cache.contains("new"));
        assert!(!cache.contains("old"), "the stale texture survived");
    }

    /// A texture bigger than the whole budget must still be usable. Refusing
    /// to shade a surface because its texture is large is a worse failure than
    /// briefly exceeding a budget, so it is served and recorded.
    #[test]
    fn a_texture_larger_than_the_budget_is_still_served() {
        let mut cache = TextureCache::new(bytes_of(4));
        let result = cache.get_or_load::<_, Never>("huge", || Ok(texture(64)));
        assert!(result.is_ok(), "an oversized texture was refused");
        assert_eq!(cache.stats().oversized, 1);

        // ...and it does not wedge the cache: the next lookup still works and
        // brings usage back under.
        let _ = cache.get_or_load::<_, Never>("small", || Ok(texture(4)));
        assert!(cache.contains("small"));
        assert!(cache.used() <= cache.budget());
    }

    /// A failed load must leave the cache exactly as it was, not half-updated.
    #[test]
    fn a_failed_load_changes_nothing() {
        let mut cache = TextureCache::new(bytes_of(16) * 4);
        let _ = cache.get_or_load::<_, Never>("good", || Ok(texture(16)));
        let before = (cache.len(), cache.used());

        let result: Result<&Texture, &str> = cache.get_or_load("bad", || Err("no such file"));
        assert!(result.is_err());
        assert_eq!(
            (cache.len(), cache.used()),
            before,
            "a failed load left the cache changed"
        );
        assert!(!cache.contains("bad"));
    }

    /// Re-inserting under the same key must replace rather than double-count,
    /// or the accounting drifts upwards until the cache evicts everything.
    #[test]
    fn replacing_a_texture_does_not_double_count() {
        let mut cache = TextureCache::new(bytes_of(32) * 4);
        cache.insert("a", texture(16));
        let after_first = cache.used();
        cache.insert("a", texture(16));

        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.used(),
            after_first,
            "re-inserting the same key counted it twice"
        );
    }

    /// The stated scenario, scaled down but with the real arithmetic: forty
    /// textures, a budget that holds a handful, and a working set that is
    /// touched repeatedly. The hit rate is what decides whether the renderer
    /// spends its time on arithmetic or on the disk.
    #[test]
    fn a_coherent_working_set_stays_resident() {
        let one = bytes_of(16);
        let mut cache = TextureCache::new(one * 8);

        // Fill with forty textures, then shade a "tile" that touches six of
        // them over and over, as rays within one screen tile do.
        for i in 0..40 {
            let _ = cache.get_or_load::<_, Never>(&format!("t{i}"), || Ok(texture(16)));
        }
        let before = cache.stats();

        for _ in 0..100 {
            for i in 0..6 {
                let _ = cache.get_or_load::<_, Never>(&format!("t{i}"), || Ok(texture(16)));
            }
        }

        let hits = cache.stats().hits - before.hits;
        let misses = cache.stats().misses - before.misses;
        let rate = hits as f64 / (hits + misses) as f64;
        assert!(
            rate > 0.95,
            "a six-texture working set in an eight-texture cache hit only \
             {:.1}% of the time",
            rate * 100.0
        );
    }

    #[test]
    fn an_empty_cache_reports_a_full_hit_rate() {
        let cache = TextureCache::new(1024);
        assert_eq!(cache.stats().hit_rate(), 1.0);
        assert!(cache.is_empty());
    }
}
