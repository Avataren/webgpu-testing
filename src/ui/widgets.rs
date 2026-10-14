//! Small egui widgets shared by the editor and the default UI windows.

use egui::{Response, Slider, SliderClamping, Ui, Widget};
use std::ops::RangeInclusive;

/// A slider tuned for fine adjustment: values move in `step` increments (no coarse
/// "smart" rounding), the track takes the available width, the number field drags one
/// step per pixel, and holding Shift while dragging the track moves ten times slower.
///
/// # Examples
///
/// ```no_run
/// # fn ui(ui: &mut wgpu_cube::ui::egui::Ui, value: &mut f32) {
/// use wgpu_cube::ui::FineSlider;
///
/// ui.add(FineSlider::new(value, 0.0..=2.0).step(0.01).suffix(" EV"));
/// # }
/// ```
pub struct FineSlider<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    logarithmic: bool,
    suffix: Option<&'a str>,
    clamp: bool,
}

impl<'a> FineSlider<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        let span = (range.end() - range.start()).abs();
        Self {
            value,
            range,
            step: default_step(span),
            logarithmic: false,
            suffix: None,
            clamp: true,
        }
    }

    /// Smallest change the slider makes.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step.max(f32::EPSILON);
        self
    }

    /// Spread small values over more of the track; good for intensities that are mostly
    /// near zero.
    pub fn logarithmic(mut self, logarithmic: bool) -> Self {
        self.logarithmic = logarithmic;
        self
    }

    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = Some(suffix);
        self
    }

    /// Let typed values go past the track's range (the track still shows the range).
    pub fn allow_beyond_range(mut self) -> Self {
        self.clamp = false;
        self
    }
}

fn default_step(span: f32) -> f32 {
    // About a thousand steps across the range, rounded to a power of ten.
    10f32.powf((span / 1000.0).max(1e-6).log10().floor())
}

fn decimals_for(step: f32) -> usize {
    (-step.log10()).ceil().clamp(0.0, 6.0) as usize
}

impl Widget for FineSlider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let FineSlider {
            value,
            range,
            step,
            logarithmic,
            suffix,
            clamp,
        } = self;
        ui.scope(|ui| {
            let reserve = 72.0;
            let width = (ui.available_width() - reserve).clamp(110.0, 360.0);
            ui.spacing_mut().slider_width = width;

            // Show "0.00", not "-0.00", for a negative zero from blending or editing.
            if *value == 0.0 {
                *value = 0.0;
            }
            let before = *value;
            let decimals = decimals_for(step);
            let mut slider = Slider::new(value, range.clone())
                .smart_aim(false)
                .logarithmic(logarithmic)
                .drag_value_speed(f64::from(step))
                .max_decimals(decimals)
                .clamping(if clamp {
                    SliderClamping::Always
                } else {
                    SliderClamping::Edits
                });
            if !logarithmic {
                slider = slider.step_by(f64::from(step));
            }
            if let Some(suffix) = suffix {
                slider = slider.suffix(suffix);
            }
            let mut response = ui.add(slider);

            // Shift-drag on the track: move by a tenth of the pointer's travel.
            if !logarithmic && response.dragged() && ui.input(|i| i.modifiers.shift) {
                let per_pixel = (range.end() - range.start()) / width;
                let delta = response.drag_delta().x * per_pixel * 0.1;
                let mut fine = before + delta;
                fine = (fine / step).round() * step;
                *value = fine.clamp(*range.start(), *range.end());
                response.mark_changed();
            }
            response
        })
        .inner
    }
}

/// Shorthand for `ui.add(FineSlider::new(value, range).step(step))`.
pub fn fine_slider(
    ui: &mut Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
) -> Response {
    ui.add(FineSlider::new(value, range).step(step))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_step_is_a_power_of_ten_well_below_the_range() {
        assert!((default_step(1.0) - 0.001).abs() < 1e-7);
        assert!((default_step(100.0) - 0.1).abs() < 1e-6);
        assert_eq!(decimals_for(0.01), 2);
        assert_eq!(decimals_for(1.0), 0);
    }
}
