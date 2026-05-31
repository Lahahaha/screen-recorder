use super::{CaptureScreenInfo, CapturedScreen, IndexedScreen};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SavedGeometry {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct GeometryInput {
    screen_index: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    saved_width: u32,
    saved_height: u32,
}

pub(super) fn single_screen_geometries(
    captured: &[CapturedScreen],
) -> BTreeMap<u32, SavedGeometry> {
    captured
        .iter()
        .map(|capture| (capture.info.index, captured_geometry(capture)))
        .collect()
}

pub(super) fn captured_geometry(capture: &CapturedScreen) -> SavedGeometry {
    SavedGeometry {
        x: 0,
        y: 0,
        width: capture.image.width(),
        height: capture.image.height(),
    }
}

pub(super) fn geometry_inputs_for_targets(
    targets: &[IndexedScreen],
    captured: &[CapturedScreen],
    scale: f32,
) -> Vec<GeometryInput> {
    targets
        .iter()
        .map(|target| {
            let captured = captured
                .iter()
                .find(|capture| capture.info.index == target.info.index);
            let (saved_width, saved_height) = captured
                .map(|capture| (capture.image.width(), capture.image.height()))
                .unwrap_or_else(|| estimated_saved_dimensions(target.info, scale));
            GeometryInput {
                screen_index: target.info.index,
                x: target.info.x,
                y: target.info.y,
                width: target.info.width.max(1),
                height: target.info.height.max(1),
                saved_width,
                saved_height,
            }
        })
        .collect()
}

fn estimated_saved_dimensions(info: CaptureScreenInfo, scale: f32) -> (u32, u32) {
    let scale = f64::from(scale.max(f32::EPSILON));
    let width = (f64::from(info.width) * info.scale_factor * scale)
        .round()
        .max(1.0) as u32;
    let height = (f64::from(info.height) * info.scale_factor * scale)
        .round()
        .max(1.0) as u32;
    (width, height)
}

pub(super) fn scaled_geometries(inputs: &[GeometryInput]) -> BTreeMap<u32, SavedGeometry> {
    let x_offsets = axis_offsets(inputs, Axis::X);
    let y_offsets = axis_offsets(inputs, Axis::Y);
    inputs
        .iter()
        .map(|input| {
            (
                input.screen_index,
                SavedGeometry {
                    x: *x_offsets.get(&input.x).unwrap_or(&0),
                    y: *y_offsets.get(&input.y).unwrap_or(&0),
                    width: input.saved_width,
                    height: input.saved_height,
                },
            )
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

fn axis_offsets(inputs: &[GeometryInput], axis: Axis) -> BTreeMap<i32, i32> {
    let mut edges = inputs
        .iter()
        .flat_map(|input| {
            let start = axis_start(*input, axis);
            let end = start + axis_logical_size(*input, axis) as i32;
            [start, end]
        })
        .collect::<Vec<_>>();
    edges.sort();
    edges.dedup();

    let mut offsets = BTreeMap::new();
    let mut current = 0_i32;
    for window in edges.windows(2) {
        let start = window[0];
        let end = window[1];
        offsets.insert(start, current);
        let distance = end.saturating_sub(start);
        let scale = axis_segment_scale(inputs, axis, start, end);
        current += (f64::from(distance) * scale).round().max(0.0) as i32;
    }
    if let Some(last) = edges.last() {
        offsets.insert(*last, current);
    }
    offsets
}

fn axis_segment_scale(inputs: &[GeometryInput], axis: Axis, start: i32, end: i32) -> f64 {
    inputs
        .iter()
        .filter_map(|input| {
            let input_start = axis_start(*input, axis);
            let input_end = input_start + axis_logical_size(*input, axis) as i32;
            if start < input_start || end > input_end {
                return None;
            }
            let logical = f64::from(axis_logical_size(*input, axis).max(1));
            let saved = f64::from(axis_saved_size(*input, axis).max(1));
            Some(saved / logical)
        })
        .reduce(f64::max)
        .unwrap_or(1.0)
}

fn axis_start(input: GeometryInput, axis: Axis) -> i32 {
    match axis {
        Axis::X => input.x,
        Axis::Y => input.y,
    }
}

fn axis_logical_size(input: GeometryInput, axis: Axis) -> u32 {
    match axis {
        Axis::X => input.width,
        Axis::Y => input.height,
    }
}

fn axis_saved_size(input: GeometryInput, axis: Axis) -> u32 {
    match axis {
        Axis::X => input.saved_width,
        Axis::Y => input.saved_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_screen_geometry_uses_saved_pixel_offsets() {
        let inputs = vec![
            GeometryInput {
                screen_index: 1,
                x: 0,
                y: 0,
                width: 1470,
                height: 956,
                saved_width: 2940,
                saved_height: 1912,
            },
            GeometryInput {
                screen_index: 2,
                x: 1470,
                y: 0,
                width: 1920,
                height: 1080,
                saved_width: 1920,
                saved_height: 1080,
            },
        ];

        let geometries = scaled_geometries(&inputs);

        assert_eq!(geometries[&1].x, 0);
        assert_eq!(geometries[&1].width, 2940);
        assert_eq!(geometries[&2].x, 2940);
        assert_eq!(geometries[&2].width, 1920);
    }
}
