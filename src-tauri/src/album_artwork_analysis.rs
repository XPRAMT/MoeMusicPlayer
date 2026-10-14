use std::{collections::HashMap, io::Cursor, sync::OnceLock};

use image::{imageops::FilterType, ImageReader};
use player_core::TrackId;

const HASH_SIZE: u32 = 32;
const LOW_FREQUENCY_COUNT: usize = 8;
const SIMILARITY_DISTANCE: u32 = 6;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtworkGroup {
    track_id: TrackId,
    hash: u64,
    count: usize,
    first_index: usize,
}

struct HashNode {
    group_index: usize,
    children: HashMap<u32, Box<HashNode>>,
}

/// Decode, hash, and rank album artwork on a backend worker. The input order comes from SQLite's
/// album/disc/track ordering; only the identifiers of the four most common covers are returned.
pub(crate) fn select_representative_artwork(
    candidates: impl IntoIterator<Item = (TrackId, Vec<u8>)>,
) -> Vec<String> {
    let mut groups = Vec::<ArtworkGroup>::new();
    let mut exact = HashMap::<u64, usize>::new();
    let mut root: Option<Box<HashNode>> = None;

    for (track_id, bytes) in candidates {
        let Some(hash) = perceptual_hash(&bytes) else {
            continue;
        };
        if let Some(index) = exact.get(&hash).copied() {
            groups[index].count += 1;
            continue;
        }

        let mut pending: Vec<&HashNode> = root.as_deref().into_iter().collect();
        let mut nearest: Option<(u32, usize)> = None;
        while let Some(node) = pending.pop() {
            let distance = hash_distance(hash, groups[node.group_index].hash);
            if distance <= SIMILARITY_DISTANCE
                && nearest.is_none_or(|(best_distance, best_index)| {
                    distance < best_distance
                        || (distance == best_distance
                            && groups[node.group_index].first_index
                                < groups[best_index].first_index)
                })
            {
                nearest = Some((distance, node.group_index));
            }
            for (edge, child) in &node.children {
                if *edge >= distance.saturating_sub(SIMILARITY_DISTANCE)
                    && *edge <= distance.saturating_add(SIMILARITY_DISTANCE)
                {
                    pending.push(child);
                }
            }
        }

        if let Some((_, index)) = nearest {
            groups[index].count += 1;
            continue;
        }

        let group_index = groups.len();
        let first_index = group_index;
        groups.push(ArtworkGroup {
            track_id,
            hash,
            count: 1,
            first_index,
        });
        exact.insert(hash, group_index);
        let inserted = Box::new(HashNode {
            group_index,
            children: HashMap::new(),
        });
        let Some(root_node) = root.as_mut() else {
            root = Some(inserted);
            continue;
        };
        let mut node = root_node.as_mut();
        loop {
            let edge = hash_distance(hash, groups[node.group_index].hash);
            match node.children.entry(edge) {
                std::collections::hash_map::Entry::Occupied(child) => {
                    node = child.into_mut();
                }
                std::collections::hash_map::Entry::Vacant(child) => {
                    child.insert(inserted);
                    break;
                }
            }
        }
    }

    groups.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.first_index.cmp(&right.first_index))
    });
    groups
        .into_iter()
        .take(4)
        .map(|group| group.track_id.to_string())
        .collect()
}

/// Find the first readable artwork without calculating album-wide representative frequencies.
pub(crate) fn first_artwork(
    candidates: impl IntoIterator<Item = (TrackId, Vec<u8>)>,
) -> Option<String> {
    candidates
        .into_iter()
        .find_map(|(track_id, bytes)| perceptual_hash(&bytes).map(|_| track_id.to_string()))
}

fn perceptual_hash(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?.to_rgba8();
    if decoded.width() == 0 || decoded.height() == 0 {
        return None;
    }

    // Canvas in the previous implementation center-cropped, fitted to 32×32, and composited
    // transparency over black. Apply the same framing and background before calculating RGB mean.
    let mut pixels = image::DynamicImage::ImageRgba8(decoded)
        .resize_to_fill(HASH_SIZE, HASH_SIZE, FilterType::Triangle)
        .to_rgba8();
    let mut gray = [0.0_f64; (HASH_SIZE * HASH_SIZE) as usize];
    for (index, pixel) in pixels.pixels_mut().enumerate() {
        let alpha = f64::from(pixel[3]) / 255.0;
        let red = (f64::from(pixel[0]) * alpha).round();
        let green = (f64::from(pixel[1]) * alpha).round();
        let blue = (f64::from(pixel[2]) * alpha).round();
        gray[index] = ((red + green + blue) / 3.0).round();
    }
    Some(hash_gray(&gray))
}

fn hash_gray(gray: &[f64; (HASH_SIZE * HASH_SIZE) as usize]) -> u64 {
    let mut mean = gray.iter().sum::<f64>() / gray.len() as f64;
    if !mean.is_finite() {
        mean = 0.0;
    }
    let variance = gray
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f64>()
        / gray.len() as f64;
    if variance < 0.01 {
        return 0;
    }

    let basis = dct_basis();
    let mut horizontal = [[0.0_f64; LOW_FREQUENCY_COUNT]; HASH_SIZE as usize];
    for y in 0..HASH_SIZE as usize {
        for u in 0..LOW_FREQUENCY_COUNT {
            for x in 0..HASH_SIZE as usize {
                horizontal[y][u] += gray[y * HASH_SIZE as usize + x] * basis[u][x];
            }
        }
    }
    let mut coefficients = [0.0_f64; LOW_FREQUENCY_COUNT * LOW_FREQUENCY_COUNT - 1];
    let mut target = 0;
    for (v, vertical_basis) in basis.iter().enumerate() {
        for (u, _) in horizontal[0].iter().enumerate() {
            if u == 0 && v == 0 {
                continue;
            }
            coefficients[target] = horizontal
                .iter()
                .zip(vertical_basis)
                .map(|(row, weight)| row[u] * weight)
                .sum();
            target += 1;
        }
    }
    let mut sorted = coefficients;
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    coefficients
        .iter()
        .enumerate()
        .fold(0_u64, |hash, (index, coefficient)| {
            if *coefficient > median {
                hash | (1_u64 << index)
            } else {
                hash
            }
        })
}

fn dct_basis() -> &'static [[f64; HASH_SIZE as usize]; LOW_FREQUENCY_COUNT] {
    static BASIS: OnceLock<[[f64; HASH_SIZE as usize]; LOW_FREQUENCY_COUNT]> = OnceLock::new();
    BASIS.get_or_init(|| {
        std::array::from_fn(|frequency| {
            std::array::from_fn(|sample| {
                (((2 * sample + 1) * frequency) as f64 * std::f64::consts::PI)
                    / (2.0 * f64::from(HASH_SIZE))
            })
            .map(f64::cos)
        })
    })
}

fn hash_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

    fn encoded_pattern(invert: bool) -> Vec<u8> {
        let mut image = RgbaImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let on = ((x / 8 + y / 8) % 2 == 0) ^ invert;
                let shade = if on { 240 } else { 16 };
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode test cover");
        bytes.into_inner()
    }

    #[test]
    fn perceptual_hash_decodes_supported_encoded_artwork() {
        let hash = perceptual_hash(&encoded_pattern(false)).expect("decode PNG");
        assert_ne!(hash, 0);
        assert_eq!(hash, perceptual_hash(&encoded_pattern(false)).unwrap());
        assert_ne!(hash, perceptual_hash(&encoded_pattern(true)).unwrap());
    }

    #[test]
    fn representative_selection_returns_only_top_four_in_frequency_order() {
        let pattern_a = encoded_pattern(false);
        let pattern_b = encoded_pattern(true);
        let first_a = TrackId::new();
        let first_b = TrackId::new();
        let candidates = [
            (first_a, pattern_a.clone()),
            (first_b, pattern_b.clone()),
            (TrackId::new(), pattern_a.clone()),
            (TrackId::new(), pattern_a.clone()),
            (TrackId::new(), pattern_b.clone()),
            (TrackId::new(), vec![0, 1, 2]),
        ];
        let ids = select_representative_artwork(candidates);
        assert_eq!(ids, [first_a.to_string(), first_b.to_string()]);
    }

    #[test]
    fn first_artwork_skips_invalid_images_and_returns_first_decodable_track() {
        let first = TrackId::new();
        let expected = TrackId::new();
        let result = first_artwork([
            (first, vec![1, 2, 3]),
            (expected, encoded_pattern(false)),
            (TrackId::new(), encoded_pattern(true)),
        ]);
        assert_eq!(result.as_deref(), Some(expected.to_string().as_str()));
    }
}
