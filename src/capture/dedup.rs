use super::{
    geometry::{captured_geometry, SavedGeometry},
    CaptureReport, CaptureState, CapturedScreen,
};
use image::DynamicImage;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use xxhash_rust::xxh3::Xxh3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CaptureBatchFingerprint {
    pub(super) output_dir: PathBuf,
    pub(super) screens: Vec<ScreenFingerprint>,
    pub(super) paths: Vec<PathBuf>,
    pub(super) metadata_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ScreenFingerprint {
    screen_index: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    hash: u64,
}

pub(super) fn duplicate_capture_report(
    state: &mut CaptureState,
    output_dir: &Path,
    screens: &[ScreenFingerprint],
    target_screen_count: usize,
    failed_screen_count: usize,
) -> Option<CaptureReport> {
    let previous = state.last_capture.as_ref()?;
    if previous.output_dir == output_dir && previous.screens == screens {
        if !previous_capture_files_exist(previous) {
            state.last_capture = None;
            return None;
        }
        return Some(CaptureReport {
            saved_paths: Vec::new(),
            previous_paths: previous.paths.clone(),
            metadata_path: previous.metadata_path.clone(),
            target_screen_count,
            failed_screen_count,
            skipped_duplicate: true,
        });
    }
    None
}

fn previous_capture_files_exist(previous: &CaptureBatchFingerprint) -> bool {
    !previous.paths.is_empty()
        && previous.paths.iter().all(|path| path.exists())
        && match &previous.metadata_path {
            Some(path) => path.exists(),
            None => true,
        }
}

pub(super) fn store_capture_fingerprint(
    state: &mut CaptureState,
    fingerprint: CaptureBatchFingerprint,
) {
    state.last_capture = Some(fingerprint);
}

pub(super) fn clear_capture_fingerprint(state: &mut CaptureState) {
    state.last_capture = None;
}

pub(super) fn screen_fingerprints(
    captured: &[CapturedScreen],
    geometries: &BTreeMap<u32, SavedGeometry>,
) -> Vec<ScreenFingerprint> {
    let mut fingerprints = captured
        .iter()
        .map(|capture| {
            let geometry = geometries
                .get(&capture.info.index)
                .copied()
                .unwrap_or_else(|| captured_geometry(capture));
            ScreenFingerprint {
                screen_index: capture.info.index,
                x: geometry.x,
                y: geometry.y,
                width: geometry.width,
                height: geometry.height,
                hash: rgba_buffer_hash(&capture.image),
            }
        })
        .collect::<Vec<_>>();
    fingerprints.sort_by_key(|fingerprint| fingerprint.screen_index);
    fingerprints
}

fn rgba_buffer_hash(image: &DynamicImage) -> u64 {
    if let Some(rgba) = image.as_rgba8() {
        return rgba_raw_hash(rgba.width(), rgba.height(), rgba.as_raw());
    }

    let rgba = image.to_rgba8();
    rgba_raw_hash(rgba.width(), rgba.height(), rgba.as_raw())
}

fn rgba_raw_hash(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut hasher = Xxh3::new();
    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    hasher.update(rgba);
    hasher.digest()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use image::RgbaImage;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-capture-dedup-test-{}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f"),
            sequence
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn rgba_buffer_hash_matches_identical_rgba_images() {
        let image = DynamicImage::ImageRgba8(
            RgbaImage::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).expect("image"),
        );
        let same = image.clone();

        assert_eq!(rgba_buffer_hash(&image), rgba_buffer_hash(&same));
    }

    #[test]
    fn rgba_buffer_hash_changes_when_pixels_change() {
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_raw(1, 1, vec![1, 2, 3, 4]).expect("image"));
        let changed =
            DynamicImage::ImageRgba8(RgbaImage::from_raw(1, 1, vec![1, 2, 3, 5]).expect("image"));

        assert_ne!(rgba_buffer_hash(&image), rgba_buffer_hash(&changed));
    }

    #[test]
    fn rgba_buffer_hash_includes_dimensions() {
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let tall =
            DynamicImage::ImageRgba8(RgbaImage::from_raw(1, 2, pixels.clone()).expect("image"));
        let wide = DynamicImage::ImageRgba8(RgbaImage::from_raw(2, 1, pixels).expect("image"));

        assert_ne!(rgba_buffer_hash(&tall), rgba_buffer_hash(&wide));
    }

    #[test]
    fn duplicate_report_matches_success_screen_set_and_geometry() {
        let dir = test_dir();
        let previous_path = dir.join("screen.png");
        fs::write(&previous_path, b"image").expect("write previous screenshot");
        let mut state = CaptureState::default();
        let screens = vec![ScreenFingerprint {
            screen_index: 1,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            hash: 42,
        }];
        store_capture_fingerprint(
            &mut state,
            CaptureBatchFingerprint {
                output_dir: dir.clone(),
                screens: screens.clone(),
                paths: vec![previous_path.clone()],
                metadata_path: None,
            },
        );

        let report = duplicate_capture_report(&mut state, &dir, &screens, 1, 0).expect("duplicate");

        assert!(report.skipped_duplicate);
        assert!(report.saved_paths.is_empty());
        assert_eq!(report.previous_paths, vec![previous_path]);
    }

    #[test]
    fn duplicate_report_ignores_stale_missing_files() {
        let dir = test_dir();
        let mut state = CaptureState::default();
        let screens = vec![ScreenFingerprint {
            screen_index: 1,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            hash: 42,
        }];
        let stale_path = dir.join("deleted-screen.png");
        store_capture_fingerprint(
            &mut state,
            CaptureBatchFingerprint {
                output_dir: dir.clone(),
                screens: screens.clone(),
                paths: vec![stale_path],
                metadata_path: None,
            },
        );

        let report = duplicate_capture_report(&mut state, &dir, &screens, 1, 0);

        assert!(report.is_none());
    }

    #[test]
    fn duplicate_report_ignores_stale_missing_metadata() {
        let dir = test_dir();
        let image_path = dir.join("screen.png");
        fs::write(&image_path, b"image").expect("write image");
        let mut state = CaptureState::default();
        let screens = vec![ScreenFingerprint {
            screen_index: 1,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            hash: 42,
        }];
        store_capture_fingerprint(
            &mut state,
            CaptureBatchFingerprint {
                output_dir: dir.clone(),
                screens: screens.clone(),
                paths: vec![image_path],
                metadata_path: Some(dir.join("deleted.screens.json")),
            },
        );

        let report = duplicate_capture_report(&mut state, &dir, &screens, 1, 0);

        assert!(report.is_none());
    }

    #[test]
    fn duplicate_report_returns_none_after_clear_capture_fingerprint() {
        let dir = test_dir();
        let previous_path = dir.join("screen.png");
        fs::write(&previous_path, b"image").expect("write previous screenshot");
        let mut state = CaptureState::default();
        let screens = vec![ScreenFingerprint {
            screen_index: 1,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            hash: 42,
        }];
        store_capture_fingerprint(
            &mut state,
            CaptureBatchFingerprint {
                output_dir: dir.clone(),
                screens: screens.clone(),
                paths: vec![previous_path],
                metadata_path: None,
            },
        );

        clear_capture_fingerprint(&mut state);

        let report = duplicate_capture_report(&mut state, &dir, &screens, 1, 0);
        assert!(report.is_none());
    }

    #[test]
    fn duplicate_state_isolated_between_capture_states() {
        let dir = test_dir();
        let previous_path = dir.join("screen.png");
        fs::write(&previous_path, b"image").expect("write previous screenshot");
        let mut first = CaptureState::default();
        let mut second = CaptureState::default();
        let screens = vec![ScreenFingerprint {
            screen_index: 1,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            hash: 42,
        }];
        store_capture_fingerprint(
            &mut first,
            CaptureBatchFingerprint {
                output_dir: dir.clone(),
                screens: screens.clone(),
                paths: vec![previous_path],
                metadata_path: None,
            },
        );

        assert!(duplicate_capture_report(&mut first, &dir, &screens, 1, 0).is_some());
        assert!(duplicate_capture_report(&mut second, &dir, &screens, 1, 0).is_none());
    }
}
