//! Comparing the frames of a `visual.capture`.
//!
//! The capture feeds every frame it takes into a [`Recorder`] as the image arrives, so only
//! a handful of frames are ever held in memory. The recorder produces three things:
//!
//! * **Differences**: the last frame of each variant against the last frame of the first
//!   variant (the reference), as statistics over a grid of regions and as an amplified
//!   difference image.
//! * **Jitter noise**: the same statistics between a variant's own consecutive frames. With
//!   temporal anti-aliasing every frame has another sub-pixel offset, so two frames of one
//!   unchanged configuration still differ a little; a real difference has to stand out from
//!   that.
//! * **Settle check**: the frame-to-frame difference over the frames rendered before the
//!   capture, which shows whether temporal history has stopped changing by the time the
//!   first image is taken.
//!
//! A pixel's difference is the largest absolute difference of its red, green and blue
//! channels, in 8-bit levels, so a change in one channel is not diluted by the others.
//!
//! Two frames of an unchanged view still differ at nearly every pixel, because the output
//! is dithered (about 0.8 levels on average in a real capture, with temporal anti-aliasing
//! on or off). That per-pixel noise swamps the mean difference and hides everything that
//! changes in few pixels or by little. So every region also gets a *smooth* difference: the
//! mean difference of 8x8 pixel block averages. Dither averages out within a block, while
//! moved edges and broad shifts of a fraction of a level do not, which makes it the number
//! to judge "is this a real difference" and "has temporal history settled" by.
//! Everything here is plain maths over [`RgbaImage`]s and is tested with synthetic images.

use std::path::PathBuf;

use image::RgbaImage;
use serde_json::{json, Map, Value};

use crate::renderer::TAA_JITTER_FRAMES;

/// Side of the pixel blocks whose averages the smooth difference compares.
const SMOOTH_BLOCK: usize = 8;
/// A difference is clearly more than jitter when it exceeds this multiple of the noise in
/// the same region, plus a margin so that a noiseless pair of frames does not flag
/// rounding-level differences.
const NOISE_FACTOR: f64 = 2.0;
const NOISE_MARGIN_SMOOTH: f64 = 0.1;
const NOISE_MARGIN_SHARE: f64 = 0.001;
/// The frame-to-frame smooth difference (levels) and share of pixels over the threshold
/// below which history counts as settled whatever its trend.
const SETTLED_LEVEL: f64 = 0.02;
const SETTLED_SHARE: f64 = 0.00005;
/// Frame-to-frame differences of an unchanged view repeat with the TAA jitter, so the
/// settle check compares whole jitter periods.
const WINDOW: usize = TAA_JITTER_FRAMES as usize;
/// History counts as settled while the last period of the difference series stays above
/// this share of the period before it (a plateau, not a decay). Real jitter noise varies
/// by a few percent between periods; history blending 10% of each frame loses over half
/// of a period's difference every 8 frames.
const PLATEAU_SHARE: f64 = 0.7;
/// The fewest differences for a settle verdict: two jitter periods.
const MIN_SETTLE_SAMPLES: usize = 2 * WINDOW;
/// How much the first captured frame may differ from the last, relative to the jitter
/// noise, before it counts as still converging.
const DRIFT_FACTOR: f64 = 1.5;
/// Regions listed in the reply, worst first.
const TOP_REGIONS: usize = 3;

/// Difference statistics of a set of pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Stats {
    pub pixels: u64,
    /// Mean difference, in levels.
    pub mean: f64,
    /// Mean difference of 8x8 block averages, in levels: the mean with per-pixel dither
    /// averaged out.
    pub smooth: f64,
    /// Largest difference, in levels.
    pub max: u8,
    /// Share of pixels whose difference is greater than the threshold, from 0 to 1.
    pub over: f64,
}

/// Running totals behind [`Stats`].
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    pixels: u64,
    total: u64,
    /// Sum over blocks of the block's pixel count times its smooth difference.
    smooth: u64,
    max: u8,
    over: u64,
}

/// Signed per-channel difference sums of the blocks across one band of rows.
type Band = Vec<[i32; 3]>;

impl Sums {
    /// Add one row of pixels; `band` collects the signed differences per block.
    fn add_row(&mut self, a: &[u8], b: &[u8], threshold: u8, band: &mut Band) {
        let blocks = a
            .chunks(SMOOTH_BLOCK * 4)
            .zip(b.chunks(SMOOTH_BLOCK * 4))
            .zip(band.iter_mut());
        for ((block_a, block_b), block) in blocks {
            for (p, q) in block_a
                .as_chunks::<4>()
                .0
                .iter()
                .zip(block_b.as_chunks::<4>().0)
            {
                let level = p[0]
                    .abs_diff(q[0])
                    .max(p[1].abs_diff(q[1]))
                    .max(p[2].abs_diff(q[2]));
                self.pixels += 1;
                self.total += u64::from(level);
                self.max = self.max.max(level);
                self.over += u64::from(level > threshold);
                for channel in 0..3 {
                    block[channel] += i32::from(p[channel]) - i32::from(q[channel]);
                }
            }
        }
    }

    /// The band of rows is complete: its blocks' averages differ by the largest channel
    /// of each block's summed difference, divided by the block's pixel count.
    fn close_band(&mut self, band: &Band) {
        for block in band {
            let largest = block.iter().map(|sum| sum.unsigned_abs()).max();
            self.smooth += u64::from(largest.unwrap_or(0));
        }
    }

    fn merge(&mut self, other: &Sums) {
        self.pixels += other.pixels;
        self.total += other.total;
        self.smooth += other.smooth;
        self.max = self.max.max(other.max);
        self.over += other.over;
    }

    fn stats(&self) -> Stats {
        if self.pixels == 0 {
            return Stats::default();
        }
        Stats {
            pixels: self.pixels,
            mean: self.total as f64 / self.pixels as f64,
            smooth: self.smooth as f64 / self.pixels as f64,
            max: self.max,
            over: self.over as f64 / self.pixels as f64,
        }
    }
}

/// Difference statistics of an image pair, by region and overall.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major: the top-left region first.
    pub cells: Vec<Stats>,
    pub whole: Stats,
}

impl Grid {
    /// The pixel offset where region `index` of `parts` regions across `len` pixels starts.
    /// Regions tile the image exactly, whatever the size, differing by at most a pixel.
    fn edge(len: u32, parts: u32, index: u32) -> u32 {
        (u64::from(index) * u64::from(len) / u64::from(parts)) as u32
    }

    /// `[x, y, width, height]` in pixels of the region at row-major `index`.
    pub fn rect(&self, index: usize) -> [u32; 4] {
        let col = index as u32 % self.cols;
        let row = index as u32 / self.cols;
        let (x0, x1) = (
            Self::edge(self.width, self.cols, col),
            Self::edge(self.width, self.cols, col + 1),
        );
        let (y0, y1) = (
            Self::edge(self.height, self.rows, row),
            Self::edge(self.height, self.rows, row + 1),
        );
        [x0, y0, x1 - x0, y1 - y0]
    }

    fn same_shape(&self, other: &Grid) -> bool {
        (self.cols, self.rows, self.width, self.height)
            == (other.cols, other.rows, other.width, other.height)
    }

    /// The mean of several grids of the same shape: means and shares are averaged, the
    /// maximum is the largest seen. Grids of another shape than the first are left out.
    pub fn average(grids: &[Grid]) -> Option<Grid> {
        let first = grids.first()?;
        let same: Vec<&Grid> = grids.iter().filter(|grid| first.same_shape(grid)).collect();
        let count = same.len() as f64;
        let combine = |pick: &dyn Fn(&Grid) -> Stats| {
            let all: Vec<Stats> = same.iter().map(|grid| pick(grid)).collect();
            Stats {
                pixels: all[0].pixels,
                mean: all.iter().map(|s| s.mean).sum::<f64>() / count,
                smooth: all.iter().map(|s| s.smooth).sum::<f64>() / count,
                max: all.iter().map(|s| s.max).max().unwrap_or(0),
                over: all.iter().map(|s| s.over).sum::<f64>() / count,
            }
        };
        Some(Grid {
            cells: (0..first.cells.len())
                .map(|index| combine(&|grid: &Grid| grid.cells[index]))
                .collect(),
            whole: combine(&|grid: &Grid| grid.whole),
            ..first.clone()
        })
    }

    /// Region by region the larger of two noise grids: the jitter a comparison of frames
    /// from two configurations has to rise above.
    fn floor(a: Option<&Grid>, b: Option<&Grid>) -> Option<Grid> {
        let larger = |x: &Stats, y: &Stats| Stats {
            pixels: x.pixels,
            mean: x.mean.max(y.mean),
            smooth: x.smooth.max(y.smooth),
            max: x.max.max(y.max),
            over: x.over.max(y.over),
        };
        match (a, b) {
            (Some(a), Some(b)) if a.same_shape(b) => Some(Grid {
                cells: a
                    .cells
                    .iter()
                    .zip(&b.cells)
                    .map(|(x, y)| larger(x, y))
                    .collect(),
                whole: larger(&a.whole, &b.whole),
                ..a.clone()
            }),
            (Some(only), None) | (None, Some(only)) => Some(only.clone()),
            _ => None,
        }
    }
}

/// Difference statistics of two images over a `cols` x `rows` grid. A pixel is over the
/// threshold when its difference is greater than `threshold` levels, so a threshold of 0
/// counts every pixel that differs at all.
pub(super) fn grid_stats(
    a: &RgbaImage,
    b: &RgbaImage,
    cols: u32,
    rows: u32,
    threshold: u8,
) -> Result<Grid, String> {
    let (width, height) = a.dimensions();
    if b.dimensions() != (width, height) {
        return Err(format!(
            "the images differ in size ({width}x{height} and {}x{})",
            b.width(),
            b.height()
        ));
    }
    if width == 0 || height == 0 {
        return Err("the images are empty".into());
    }
    let cols = cols.clamp(1, width);
    let rows = rows.clamp(1, height);
    let stride = width as usize * 4;
    let (raw_a, raw_b) = (a.as_raw(), b.as_raw());

    let mut sums = Vec::with_capacity((cols * rows) as usize);
    for row in 0..rows {
        let (y0, y1) = (
            Grid::edge(height, rows, row),
            Grid::edge(height, rows, row + 1),
        );
        for col in 0..cols {
            let (x0, x1) = (
                Grid::edge(width, cols, col),
                Grid::edge(width, cols, col + 1),
            );
            let mut cell = Sums::default();
            let mut band: Band = vec![[0; 3]; ((x1 - x0) as usize).div_ceil(SMOOTH_BLOCK)];
            let mut y = y0;
            while y < y1 {
                let band_end = (y + SMOOTH_BLOCK as u32).min(y1);
                band.fill([0; 3]);
                for row_y in y..band_end {
                    let start = row_y as usize * stride + x0 as usize * 4;
                    let end = row_y as usize * stride + x1 as usize * 4;
                    cell.add_row(&raw_a[start..end], &raw_b[start..end], threshold, &mut band);
                }
                cell.close_band(&band);
                y = band_end;
            }
            sums.push(cell);
        }
    }
    let mut whole = Sums::default();
    for cell in &sums {
        whole.merge(cell);
    }
    Ok(Grid {
        cols,
        rows,
        width,
        height,
        cells: sums.iter().map(Sums::stats).collect(),
        whole: whole.stats(),
    })
}

/// A difference of `level` levels multiplied by `gain` and clamped to what 8 bits hold.
pub(super) fn amplify(level: u8, gain: f32) -> u8 {
    (f32::from(level) * gain).round().clamp(0.0, 255.0) as u8
}

/// The absolute difference of two images per colour channel, multiplied by `gain` so that
/// small differences show. Equal pixels are black; the result is opaque.
pub(super) fn diff_image(a: &RgbaImage, b: &RgbaImage, gain: f32) -> Result<RgbaImage, String> {
    if a.dimensions() != b.dimensions() {
        return Err("the images differ in size".into());
    }
    let mut out = RgbaImage::new(a.width(), a.height());
    for ((p, q), pixel) in a.pixels().zip(b.pixels()).zip(out.pixels_mut()) {
        for channel in 0..3 {
            pixel.0[channel] = amplify(p.0[channel].abs_diff(q.0[channel]), gain);
        }
        pixel.0[3] = 255;
    }
    Ok(out)
}

/// How far `diff` is above the jitter `floor` of the same pixels, as a multiple of the
/// limit it has to exceed: above 1 is clearly more than jitter. Either the smooth
/// difference (broad or faint changes) or the share of pixels over the threshold (small
/// strong changes) can be the one that stands out; the larger counts.
pub(super) fn noise_score(diff: &Stats, floor: &Stats) -> f64 {
    let smooth = diff.smooth / (NOISE_FACTOR * floor.smooth + NOISE_MARGIN_SMOOTH);
    let share = diff.over / (NOISE_FACTOR * floor.over + NOISE_MARGIN_SHARE);
    smooth.max(share)
}

/// How much two consecutive frames differ.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Level {
    /// Smooth difference in levels: broad or faint changes.
    pub smooth: f64,
    /// Share of pixels over the threshold, 0 to 1: strong changes in few pixels.
    pub over: f64,
}

/// How one kind of frame-to-frame difference ended: the means of the last jitter period of a
/// series and of the period before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Trend {
    pub late: f64,
    pub earlier: f64,
    /// Not falling any more: a plateau, or too small to matter.
    pub settled: bool,
}

/// Whether temporal history had stopped changing, judged from how much consecutive frames
/// differ in two ways, either of which can still be moving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Settle {
    pub converged: bool,
    pub smooth: Trend,
    pub over: Trend,
}

/// Judge a series of frame-to-frame differences, oldest first. History that is still
/// converging shows differences that keep falling; once it has converged they stay at the
/// jitter noise level, however high that is. The last period of the jitter pattern is compared
/// with the one before it, so the pattern itself does not look like a trend. A series below
/// `floor` is settled whatever its shape. `None` when there are fewer than two periods.
fn assess_trend(series: &[f64], floor: f64) -> Option<Trend> {
    if series.len() < MIN_SETTLE_SAMPLES {
        return None;
    }
    let mean = |part: &[f64]| part.iter().sum::<f64>() / part.len() as f64;
    let end = series.len();
    let late = mean(&series[end - WINDOW..]);
    let earlier = mean(&series[end - 2 * WINDOW..end - WINDOW]);
    Some(Trend {
        late,
        earlier,
        settled: late <= floor || late >= PLATEAU_SHARE * earlier,
    })
}

/// Judge consecutive-frame differences, oldest first, by both the smooth difference and the
/// share of pixels over the threshold; history has settled when neither is still falling.
pub(super) fn assess_settle(series: &[Level]) -> Option<Settle> {
    let smooth: Vec<f64> = series.iter().map(|level| level.smooth).collect();
    let over: Vec<f64> = series.iter().map(|level| level.over).collect();
    let smooth = assess_trend(&smooth, SETTLED_LEVEL)?;
    let over = assess_trend(&over, SETTLED_SHARE)?;
    Some(Settle {
        converged: smooth.settled && over.settled,
        smooth,
        over,
    })
}

/// What the capture asks of the comparison.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Options {
    pub cols: u32,
    pub rows: u32,
    pub threshold: u8,
    pub gain: f32,
    /// Images captured per variant, after settling.
    pub frames: u64,
    /// Settle frames read back per variant, for the settle check.
    pub settle_frames: u64,
}

/// Why a frame was captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameKind {
    /// Rendered while the variant settled; only used for the settle check.
    Settle,
    /// One of the variant's saved images.
    Image,
}

struct Previous {
    image: RgbaImage,
    frame: u64,
    kind: FrameKind,
}

/// The frames of the variant being captured.
struct Stream {
    variant: usize,
    previous: Option<Previous>,
    /// The first saved image, kept when there are enough frames to compare it with the last.
    first: Option<RgbaImage>,
    images: u64,
}

#[derive(Default)]
struct VariantResult {
    /// Difference between consecutive frames from the settle period up to and including
    /// the first saved image.
    trace: Vec<Level>,
    /// Statistics between consecutive saved images (jitter noise).
    spread: Vec<Grid>,
    /// Mean difference between the first and the last saved image.
    drift: Option<f64>,
    /// Last image against the reference's last image.
    diff: Option<Grid>,
    error: Option<String>,
    diff_image: Option<PathBuf>,
    noise_image: Option<PathBuf>,
}

/// Collects the frames of a capture and compares them as they arrive.
pub(super) struct Recorder {
    options: Options,
    dir: Option<PathBuf>,
    names: Vec<String>,
    results: Vec<VariantResult>,
    stream: Option<Stream>,
    /// The last saved image of the first variant.
    reference: Option<RgbaImage>,
    warnings: Vec<String>,
}

/// The finished comparison: how it was set up, one entry per variant (for the reply and for
/// the manifest) and anything the caller should be warned about.
pub(super) struct Outcome {
    pub options: Value,
    pub variants: Vec<(Value, Value)>,
    pub warnings: Vec<String>,
}

impl Recorder {
    /// `dir` is where difference images go; without one none are written.
    pub fn new(options: Options, names: Vec<String>, dir: Option<PathBuf>) -> Self {
        let results = names.iter().map(|_| VariantResult::default()).collect();
        Self {
            options,
            dir,
            names,
            results,
            stream: None,
            reference: None,
            warnings: Vec::new(),
        }
    }

    /// Take the next frame of `variant`. Frames of one variant arrive in order, and the
    /// variants one after the other; the first variant is the reference.
    pub fn push(&mut self, variant: usize, kind: FrameKind, frame: u64, image: RgbaImage) {
        let mut stream = match self.stream.take() {
            Some(stream) if stream.variant == variant => stream,
            _ => Stream {
                variant,
                previous: None,
                first: None,
                images: 0,
            },
        };
        let last = kind == FrameKind::Image && stream.images + 1 == self.options.frames;
        if let Some(previous) = &stream.previous {
            self.compare_with_previous(variant, previous, kind, frame, &image, last);
        }
        if kind == FrameKind::Image {
            stream.images += 1;
            if stream.images == 1 && self.options.frames >= 3 {
                stream.first = Some(image.clone());
            }
            if last {
                self.finish_variant(variant, &stream, &image);
            }
        }
        stream.previous = Some(Previous { image, frame, kind });
        self.stream = Some(stream);
    }

    fn compare_with_previous(
        &mut self,
        variant: usize,
        previous: &Previous,
        kind: FrameKind,
        frame: u64,
        image: &RgbaImage,
        last: bool,
    ) {
        let name = self.names[variant].clone();
        if previous.frame + 1 != frame {
            self.warn(format!(
                "variant {name}: frames {} and {frame} are not consecutive (the engine skipped \
                 frames), so their difference is more than one frame of jitter",
                previous.frame
            ));
        }
        let both_saved = previous.kind == FrameKind::Image && kind == FrameKind::Image;
        // Settle frames only need the overall level, so skip the regions.
        let (cols, rows) = if both_saved {
            (self.options.cols, self.options.rows)
        } else {
            (1, 1)
        };
        let stats = match grid_stats(&previous.image, image, cols, rows, self.options.threshold) {
            Ok(stats) => stats,
            Err(err) => {
                self.warn(format!(
                    "variant {name}: consecutive frames not compared: {err}"
                ));
                return;
            }
        };
        if !both_saved {
            self.results[variant].trace.push(Level {
                smooth: stats.whole.smooth,
                over: stats.whole.over,
            });
            return;
        }
        self.results[variant].spread.push(stats);
        if last {
            let path = self.write_image(&previous.image, image, &name, "noise");
            self.results[variant].noise_image = path;
        }
    }

    /// The variant's last image is in: compare it with the reference and its first image.
    fn finish_variant(&mut self, variant: usize, stream: &Stream, last: &RgbaImage) {
        let name = self.names[variant].clone();
        if let Some(first) = &stream.first {
            match grid_stats(first, last, 1, 1, self.options.threshold) {
                Ok(stats) => self.results[variant].drift = Some(stats.whole.smooth),
                Err(err) => self.warn(format!("variant {name}: first and last image: {err}")),
            }
        }
        if variant == 0 {
            self.reference = Some(last.clone());
            return;
        }
        let Some(reference) = self.reference.take() else {
            self.results[variant].error = Some("there is no reference image".into());
            return;
        };
        let Options {
            cols,
            rows,
            threshold,
            ..
        } = self.options;
        match grid_stats(&reference, last, cols, rows, threshold) {
            Ok(grid) => {
                self.results[variant].diff = Some(grid);
                let path = self.write_image(&reference, last, &name, "diff");
                self.results[variant].diff_image = path;
            }
            Err(err) => {
                self.warn(format!(
                    "variant {name} not compared with {}: {err}",
                    self.names[0]
                ));
                self.results[variant].error = Some(err);
            }
        }
        self.reference = Some(reference);
    }

    /// Write the amplified difference of two images as `<name>_<suffix>.png`.
    fn write_image(
        &mut self,
        a: &RgbaImage,
        b: &RgbaImage,
        name: &str,
        suffix: &str,
    ) -> Option<PathBuf> {
        let path = self.dir.as_ref()?.join(format!("{name}_{suffix}.png"));
        let written = diff_image(a, b, self.options.gain)
            .and_then(|image| image.save(&path).map_err(|err| err.to_string()));
        match written {
            Ok(()) => Some(path),
            Err(err) => {
                self.warn(format!("failed to write {}: {err}", path.display()));
                None
            }
        }
    }

    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    /// Everything gathered, as JSON and warnings.
    pub fn finish(mut self) -> Outcome {
        let reference_name = self.names[0].clone();
        let reference_noise = Grid::average(&self.results[0].spread);
        if self.options.frames < 2 {
            self.warn(
                "frames is 1, so there is no jitter noise to tell real differences from; use \
                 frames of 2 or more"
                    .into(),
            );
        }

        let mut variants = Vec::new();
        for index in 0..self.names.len() {
            let analysis = self.analyse(index, reference_noise.as_ref());
            let entry = |detail: bool| {
                json!({
                    "compare": analysis.compare_json(&self.options, detail),
                    "settle": analysis.settle_json(&self.options, detail),
                })
            };
            variants.push((entry(false), entry(true)));
            let notes = analysis.warnings(&self.names[index], &reference_name);
            for note in notes {
                self.warn(note);
            }
        }
        Outcome {
            options: json!({
                "reference": reference_name,
                "grid": [self.options.cols, self.options.rows],
                "threshold": self.options.threshold,
                "diff_gain": self.options.gain,
            }),
            variants,
            warnings: self.warnings,
        }
    }

    fn analyse(&self, index: usize, reference_noise: Option<&Grid>) -> Analysis<'_> {
        let result = &self.results[index];
        let noise = Grid::average(&result.spread);
        let mut analysis = Analysis {
            index,
            result,
            noise,
            floor: None,
            scores: Vec::new(),
            score: None,
            differs: None,
            series: result
                .trace
                .iter()
                .copied()
                .chain(result.spread.iter().map(|grid| Level {
                    smooth: grid.whole.smooth,
                    over: grid.whole.over,
                }))
                .collect(),
            settle: None,
        };
        analysis.settle = assess_settle(&analysis.series);
        if index == 0 {
            return analysis;
        }
        let Some(diff) = &result.diff else {
            return analysis;
        };
        let floor = Grid::floor(analysis.noise.as_ref(), reference_noise);
        let zero = Stats::default();
        let floor_of = |cell: usize| {
            floor
                .as_ref()
                .and_then(|floor| floor.cells.get(cell))
                .unwrap_or(&zero)
        };
        analysis.scores = diff
            .cells
            .iter()
            .enumerate()
            .map(|(cell, stats)| noise_score(stats, floor_of(cell)))
            .collect();
        let whole_floor = floor.as_ref().map_or(zero, |floor| floor.whole);
        let score = noise_score(&diff.whole, &whole_floor);
        analysis.score = Some(score);
        analysis.differs = Some(score > 1.0 || analysis.scores.iter().any(|&score| score > 1.0));
        analysis.floor = floor;
        analysis
    }
}

/// What the comparison found for one variant.
struct Analysis<'a> {
    index: usize,
    result: &'a VariantResult,
    noise: Option<Grid>,
    /// The jitter the difference to the reference has to rise above.
    floor: Option<Grid>,
    /// Per region of the difference grid: [`noise_score`] against the jitter floor.
    scores: Vec<f64>,
    /// The same for the whole image.
    score: Option<f64>,
    differs: Option<bool>,
    /// Frame-to-frame differences, from the settle period to the last image.
    series: Vec<Level>,
    settle: Option<Settle>,
}

impl Analysis<'_> {
    /// Whether the first saved image still differs from the last by more than jitter.
    fn drifting(&self) -> bool {
        match (self.result.drift, &self.noise) {
            (Some(drift), Some(noise)) => {
                drift > DRIFT_FACTOR * noise.whole.smooth + NOISE_MARGIN_SMOOTH
            }
            _ => false,
        }
    }

    fn warnings(&self, name: &str, reference: &str) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(settle) = self.settle.filter(|settle| !settle.converged) {
            let mut moving = Vec::new();
            if !settle.smooth.settled {
                moving.push(format!(
                    "the smooth difference was still {:.3} levels at the end, down from {:.3} \
                     before",
                    settle.smooth.late, settle.smooth.earlier
                ));
            }
            if !settle.over.settled {
                moving.push(format!(
                    "{:.4}% of pixels were still changing by more than the threshold, down \
                     from {:.4}% before",
                    settle.over.late * 100.0,
                    settle.over.earlier * 100.0
                ));
            }
            notes.push(format!(
                "variant {name}: temporal history had not settled: {}; raise settle_frames",
                moving.join(", and ")
            ));
        }
        if self.drifting() {
            notes.push(format!(
                "variant {name}: the first saved image still differs from the last by {:.3} \
                 levels (smooth), against {:.3} between consecutive frames; history was \
                 still converging, raise settle_frames",
                self.result.drift.unwrap_or_default(),
                self.noise.as_ref().map_or(0.0, |noise| noise.whole.smooth)
            ));
        }
        if self.index > 0 && self.differs.is_none() && self.result.error.is_none() {
            notes.push(format!("variant {name} was not compared with {reference}"));
        }
        notes
    }

    fn compare_json(&self, options: &Options, detail: bool) -> Value {
        let result = self.result;
        let mut out = Map::new();
        if self.index == 0 {
            out.insert("role".into(), json!("reference"));
        }
        if let Some(error) = &result.error {
            out.insert("error".into(), json!(error));
        }
        if let Some(diff) = &result.diff {
            merge(&mut out, stats_json(&diff.whole));
            out.insert("differs".into(), json!(self.differs));
            out.insert("score".into(), json!(self.score.map(r2)));
            let flagged = self.scores.iter().filter(|&&score| score > 1.0).count();
            out.insert(
                "regions_above_noise".into(),
                json!(format!("{flagged} of {}", diff.cells.len())),
            );
            if flagged > 0 {
                out.insert(
                    "top_regions".into(),
                    Value::Array(self.top_regions(diff, TOP_REGIONS)),
                );
            }
        }
        out.insert(
            "noise".into(),
            self.noise
                .as_ref()
                .map_or(Value::Null, |noise| stats_json(&noise.whole)),
        );
        for (key, path) in [
            ("diff_image", &result.diff_image),
            ("noise_image", &result.noise_image),
        ] {
            if let Some(path) = path {
                out.insert(key.into(), json!(path.display().to_string()));
            }
        }
        if detail {
            if let Some(diff) = &result.diff {
                out.insert(
                    "regions".into(),
                    self.regions_json(diff, self.floor.as_ref(), Some(&self.scores)),
                );
            }
            if let Some(noise) = &self.noise {
                out.insert("noise_regions".into(), self.regions_json(noise, None, None));
            }
            out.insert("threshold".into(), json!(options.threshold));
            out.insert(
                "meaning".into(),
                json!(
                    "difference per pixel = largest absolute R, G or B channel difference \
                     (8-bit levels); over_pct = share of pixels over the threshold; noise = \
                     the variant's own consecutive saved frames (dither and TAA jitter); \
                     smooth = mean difference of 8x8 block averages, which averages dither \
                     out; score = the larger of smooth / (2 x noise smooth + 0.1) and \
                     over_pct / (2 x noise over_pct + 0.1), using the larger noise of this \
                     variant and the reference; above_noise = score > 1"
                ),
            );
        }
        Value::Object(out)
    }

    fn top_regions(&self, diff: &Grid, count: usize) -> Vec<Value> {
        let mut order: Vec<usize> = (0..diff.cells.len()).collect();
        let score = |cell: usize| self.scores.get(cell).copied().unwrap_or(0.0);
        // The share over the threshold saturates when every pixel is over it, so regions
        // tie on score then; the smooth difference tells them apart.
        let smooth = |cell: usize| diff.cells[cell].smooth;
        order.sort_by(|&a, &b| {
            score(b)
                .total_cmp(&score(a))
                .then(smooth(b).total_cmp(&smooth(a)))
        });
        order
            .into_iter()
            .filter(|&cell| score(cell) > 1.0)
            .take(count)
            .map(|cell| region_json(diff, cell, None, self.scores.get(cell).copied()))
            .collect()
    }

    fn regions_json(&self, grid: &Grid, floor: Option<&Grid>, scores: Option<&[f64]>) -> Value {
        Value::Array(
            (0..grid.cells.len())
                .map(|cell| {
                    let floor_cell = floor.and_then(|floor| floor.cells.get(cell));
                    region_json(
                        grid,
                        cell,
                        floor_cell,
                        scores.and_then(|scores| scores.get(cell).copied()),
                    )
                })
                .collect(),
        )
    }

    fn settle_json(&self, options: &Options, detail: bool) -> Value {
        let mut out = Map::new();
        match self.settle {
            Some(settle) => {
                out.insert("converged".into(), json!(settle.converged));
                out.insert(
                    "smooth".into(),
                    json!({"late": r4(settle.smooth.late), "earlier": r4(settle.smooth.earlier)}),
                );
                out.insert(
                    "over_pct".into(),
                    json!({
                        "late": r4(settle.over.late * 100.0),
                        "earlier": r4(settle.over.earlier * 100.0),
                    }),
                );
            }
            None => {
                out.insert("converged".into(), Value::Null);
                out.insert(
                    "note".into(),
                    json!(format!(
                        "no verdict: {MIN_SETTLE_SAMPLES} frame-to-frame differences (two TAA \
                         jitter periods) are needed and there were {} ({} settle frames \
                         compared, {} saved images); use settle_check with settle_frames of {} \
                         or more, or raise frames",
                        self.series.len(),
                        options.settle_frames,
                        options.frames,
                        MIN_SETTLE_SAMPLES - 1
                    )),
                );
            }
        }
        out.insert("frames_compared".into(), json!(self.series.len()));
        if let Some(drift) = self.result.drift {
            out.insert("first_to_last".into(), json!(r4(drift)));
            out.insert("drifting".into(), json!(self.drifting()));
        }
        if detail {
            let list = |pick: &dyn Fn(&Level) -> f64| {
                json!(self
                    .series
                    .iter()
                    .map(|level| r4(pick(level)))
                    .collect::<Vec<_>>())
            };
            out.insert("frame_to_frame".into(), list(&|level| level.smooth));
            out.insert(
                "frame_to_frame_over_pct".into(),
                list(&|level| level.over * 100.0),
            );
            out.insert(
                "meaning".into(),
                json!(
                    "difference between consecutive frames, oldest first: the last frames \
                     rendered while settling, then the saved images; frame_to_frame is the \
                     smooth difference in levels, frame_to_frame_over_pct the share of pixels \
                     over the threshold; late and earlier are the means of the last 8 frames \
                     (one TAA jitter period) and the 8 before; each is settled when late stays \
                     above 70% of earlier (a plateau rather than a decay) or is below 0.02 \
                     levels / 0.005%, and converged needs both"
                ),
            );
        }
        Value::Object(out)
    }
}

fn merge(out: &mut Map<String, Value>, stats: Value) {
    if let Value::Object(stats) = stats {
        out.extend(stats);
    }
}

fn r2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn r4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn stats_json(stats: &Stats) -> Value {
    json!({
        "mean": r4(stats.mean),
        "smooth": r4(stats.smooth),
        "max": stats.max,
        "over_pct": r4(stats.over * 100.0),
    })
}

fn region_json(grid: &Grid, cell: usize, noise: Option<&Stats>, score: Option<f64>) -> Value {
    let mut out = Map::new();
    out.insert(
        "cell".into(),
        json!([cell as u32 % grid.cols, cell as u32 / grid.cols]),
    );
    out.insert("rect".into(), json!(grid.rect(cell)));
    merge(&mut out, stats_json(&grid.cells[cell]));
    if let Some(noise) = noise {
        out.insert("noise_smooth".into(), json!(r4(noise.smooth)));
        out.insert("noise_over_pct".into(), json!(r4(noise.over * 100.0)));
    }
    if let Some(score) = score {
        out.insert("score".into(), json!(r2(score)));
        out.insert("above_noise".into(), json!(score > 1.0));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn flat(width: u32, height: u32, value: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([value, value, value, 255]))
    }

    fn with_pixels(mut image: RgbaImage, pixels: &[(u32, u32, [u8; 3])]) -> RgbaImage {
        for &(x, y, [r, g, b]) in pixels {
            image.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
        image
    }

    #[test]
    fn identical_images_have_no_difference() {
        let image = flat(8, 8, 90);
        let grid = grid_stats(&image, &image, 4, 4, 4).unwrap();
        assert_eq!(grid.cells.len(), 16);
        assert_eq!(grid.whole.mean, 0.0);
        assert_eq!(grid.whole.max, 0);
        assert_eq!(grid.whole.over, 0.0);
        assert!(grid.cells.iter().all(|cell| cell.pixels == 4));
    }

    #[test]
    fn a_changed_pixel_lands_in_its_region_only() {
        let a = flat(8, 8, 100);
        // Pixel (5, 2) is in column 2 (x 4..6) and row 1 (y 2..4) of a 4x4 grid of 2x2
        // regions.
        let b = with_pixels(a.clone(), &[(5, 2, [140, 100, 100])]);
        let grid = grid_stats(&a, &b, 4, 4, 4).unwrap();
        for (index, cell) in grid.cells.iter().enumerate() {
            if index == 4 + 2 {
                assert_eq!(cell.max, 40);
                assert_eq!(cell.mean, 10.0, "40 levels over four pixels");
                assert_eq!(cell.over, 0.25);
            } else {
                assert_eq!(cell.max, 0, "region {index}");
            }
        }
        assert_eq!(grid.whole.max, 40);
        assert!((grid.whole.mean - 40.0 / 64.0).abs() < 1e-12);
        assert!((grid.whole.over - 1.0 / 64.0).abs() < 1e-12);
        assert_eq!(grid.rect(6), [4, 2, 2, 2]);
    }

    #[test]
    fn the_difference_is_the_largest_channel_not_the_average() {
        let a = flat(2, 2, 50);
        let b = with_pixels(a.clone(), &[(0, 0, [50, 50, 80])]);
        let grid = grid_stats(&a, &b, 1, 1, 0).unwrap();
        assert_eq!(grid.whole.max, 30, "only blue changed, by 30");
        assert_eq!(grid.whole.mean, 7.5);
    }

    #[test]
    fn the_threshold_counts_strictly_greater_differences() {
        let a = flat(4, 1, 100);
        let b = with_pixels(
            a.clone(),
            &[(0, 0, [104; 3]), (1, 0, [105; 3]), (2, 0, [101; 3])],
        );
        let at_4 = grid_stats(&a, &b, 1, 1, 4).unwrap();
        assert_eq!(at_4.whole.over, 0.25, "only the 5-level pixel is over 4");
        let at_0 = grid_stats(&a, &b, 1, 1, 0).unwrap();
        assert_eq!(at_0.whole.over, 0.75, "threshold 0 counts any difference");
        let at_5 = grid_stats(&a, &b, 1, 1, 5).unwrap();
        assert_eq!(at_5.whole.over, 0.0);
    }

    #[test]
    fn regions_tile_images_that_do_not_divide_evenly() {
        let a = flat(10, 7, 0);
        let b = flat(10, 7, 10);
        let grid = grid_stats(&a, &b, 4, 3, 4).unwrap();
        assert_eq!(grid.cells.len(), 12);
        let covered: u64 = grid.cells.iter().map(|cell| cell.pixels).sum();
        assert_eq!(covered, 70, "every pixel is in exactly one region");
        assert_eq!(grid.whole.pixels, 70);
        assert!(grid
            .cells
            .iter()
            .all(|cell| cell.mean == 10.0 && cell.over == 1.0));
        let rects: u32 = (0..12).map(|i| grid.rect(i)[2] * grid.rect(i)[3]).sum();
        assert_eq!(rects, 70);
        assert_eq!(grid.rect(11), [7, 4, 3, 3], "rows split 2, 2, 3");
    }

    #[test]
    fn a_grid_finer_than_the_image_is_clamped() {
        let image = flat(3, 2, 0);
        let grid = grid_stats(&image, &image, 16, 16, 4).unwrap();
        assert_eq!((grid.cols, grid.rows), (3, 2));
        assert_eq!(grid.cells.len(), 6);
    }

    #[test]
    fn images_of_different_sizes_are_an_error() {
        let err = grid_stats(&flat(4, 4, 0), &flat(4, 5, 0), 2, 2, 4).unwrap_err();
        assert!(err.contains("4x4 and 4x5"), "{err}");
        assert!(diff_image(&flat(4, 4, 0), &flat(5, 4, 0), 2.0).is_err());
    }

    #[test]
    fn amplification_scales_and_saturates() {
        assert_eq!(amplify(0, 16.0), 0);
        assert_eq!(amplify(3, 16.0), 48);
        assert_eq!(amplify(3, 1.0), 3);
        assert_eq!(amplify(15, 16.0), 240);
        assert_eq!(amplify(16, 16.0), 255);
        assert_eq!(amplify(200, 16.0), 255, "clamped, not wrapped");
        assert_eq!(amplify(1, 0.5), 1, "rounds half up");
    }

    #[test]
    fn the_difference_image_is_per_channel_amplified_and_opaque() {
        let a = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 7]));
        let b = with_pixels(a.clone(), &[(1, 0, [12, 20, 40])]);
        let out = diff_image(&a, &b, 10.0).unwrap();
        assert_eq!(
            out.get_pixel(0, 0).0,
            [0, 0, 0, 255],
            "equal pixels are black"
        );
        assert_eq!(out.get_pixel(1, 0).0, [20, 0, 100, 255]);
        // Order does not matter: it is an absolute difference.
        assert_eq!(diff_image(&b, &a, 10.0).unwrap(), out);
    }

    #[test]
    fn averaging_grids_averages_means_and_keeps_the_worst_maximum() {
        let base = flat(4, 4, 0);
        let one = grid_stats(&base, &flat(4, 4, 2), 2, 2, 4).unwrap();
        let two = grid_stats(
            &base,
            &with_pixels(flat(4, 4, 0), &[(0, 0, [60; 3]), (1, 0, [60; 3])]),
            2,
            2,
            4,
        )
        .unwrap();
        let average = Grid::average(&[one.clone(), two.clone()]).unwrap();
        assert_eq!(average.cells[0].max, 60);
        assert_eq!(average.cells[3].max, 2);
        assert_eq!(average.cells[0].mean, (2.0 + 30.0) / 2.0);
        assert_eq!(
            average.cells[0].over, 0.25,
            "half the pixels in one of two grids"
        );
        assert_eq!(average.whole.mean, (2.0 + 7.5) / 2.0);
        assert!(Grid::average(&[]).is_none());
        // A grid of another shape is not mixed in.
        let other = grid_stats(&base, &flat(4, 4, 9), 1, 1, 4).unwrap();
        assert_eq!(Grid::average(&[one.clone(), other]).unwrap(), one);
    }

    fn stats(smooth: f64, over: f64) -> Stats {
        Stats {
            pixels: 100,
            mean: 0.8,
            smooth,
            max: 9,
            over,
        }
    }

    #[test]
    fn differences_need_to_clear_the_noise_to_count() {
        let noise = stats(0.2, 0.01);
        let quiet = Stats::default();
        let above = |diff: Stats, floor: &Stats| noise_score(&diff, floor) > 1.0;
        // Limits: 2 x 0.2 + 0.1 = 0.5 levels smooth, or 2 x 1% + 0.1% = 2.1% of pixels.
        assert!(!above(stats(0.4, 0.02), &noise));
        assert!(above(stats(0.6, 0.0), &noise));
        assert!(above(stats(0.0, 0.03), &noise));
        // The score is how many times over the limit the larger signal is.
        assert!((noise_score(&stats(1.0, 0.0), &noise) - 2.0).abs() < 1e-12);
        assert!((noise_score(&stats(0.0, 0.105), &noise) - 5.0).abs() < 1e-9);
        // With no noise at all, a rounding-level difference is still not a difference.
        assert!(!above(stats(0.05, 0.0), &quiet));
        assert!(above(stats(0.2, 0.0), &quiet));
        // The per-pixel mean does not count: dither makes it large for any pair of frames.
        let dithered = Stats {
            mean: 5.0,
            ..Stats::default()
        };
        assert!(!above(dithered, &quiet));
    }

    #[test]
    fn smoothing_averages_per_pixel_noise_out_but_keeps_broad_shifts() {
        // A checkerboard of +-3 around a flat level: every pixel differs from the flat
        // image by 3, but each 8x8 block averages to the same value as before.
        let flat_image = flat(16, 16, 100);
        let checker = RgbaImage::from_fn(16, 16, |x, y| {
            let value = if (x + y) % 2 == 0 { 103 } else { 97 };
            Rgba([value, value, value, 255])
        });
        let noisy = grid_stats(&flat_image, &checker, 1, 1, 0).unwrap().whole;
        assert_eq!(noisy.mean, 3.0);
        assert_eq!(noisy.smooth, 0.0, "dither is not a difference");

        // A shift of one level everywhere is as small as the noise per pixel, but it is
        // the same in every block.
        let shifted = grid_stats(&flat_image, &flat(16, 16, 101), 1, 1, 4)
            .unwrap()
            .whole;
        assert_eq!(shifted.smooth, 1.0);

        // Noise on top of a shift leaves the shift.
        let both = RgbaImage::from_fn(16, 16, |x, y| {
            let value = if (x + y) % 2 == 0 { 104 } else { 98 };
            Rgba([value, value, value, 255])
        });
        let both = grid_stats(&flat_image, &both, 1, 1, 4).unwrap().whole;
        assert_eq!(both.smooth, 1.0);
        assert_eq!(both.mean, 3.0, "the mean of |+4| and |-2| is 3");
    }

    #[test]
    fn smoothing_weights_edge_blocks_by_their_pixels_and_follows_the_largest_channel() {
        // 10x7 is one full block and three partial ones per band; a flat difference of 10
        // is 10 in every block whatever its size.
        let flat_diff = grid_stats(&flat(10, 7, 0), &flat(10, 7, 10), 1, 1, 4).unwrap();
        assert_eq!(flat_diff.whole.smooth, 10.0);

        // Only the first 8x8 block of a 16x8 image changes, by 40 in one channel: half
        // the pixels, so the smooth mean is 20.
        let a = flat(16, 8, 50);
        let mut b = a.clone();
        for y in 0..8 {
            for x in 0..8 {
                b.put_pixel(x, y, Rgba([50, 90, 50, 255]));
            }
        }
        let grid = grid_stats(&a, &b, 2, 1, 4).unwrap();
        assert_eq!(grid.cells[0].smooth, 40.0);
        assert_eq!(grid.cells[1].smooth, 0.0);
        assert_eq!(grid.whole.smooth, 20.0);
    }

    /// A series where both signals follow `smooth`, scaled for the share.
    fn levels(smooth: &[f64]) -> Vec<Level> {
        smooth
            .iter()
            .map(|&smooth| Level {
                smooth,
                over: smooth / 1000.0,
            })
            .collect()
    }

    #[test]
    fn falling_differences_are_not_settled_but_a_plateau_is() {
        // Temporal history blending 10% of each new frame decays by 0.9 a frame.
        let decaying: Vec<f64> = (0..18).map(|i| 8.0 * 0.9f64.powi(i)).collect();
        let verdict = assess_settle(&levels(&decaying)).unwrap();
        assert!(!verdict.converged);
        assert!(!verdict.smooth.settled && !verdict.over.settled);
        assert!(verdict.smooth.late < verdict.smooth.earlier);

        // The same decay that has run its course settles onto a jitter level.
        let settled: Vec<f64> = (0..18).map(|i| 1.5 + 8.0 * 0.5f64.powi(i)).collect();
        assert!(assess_settle(&levels(&settled)).unwrap().converged);

        let flat_noise: Vec<f64> = (0..17).map(|i| 0.8 + 0.05 * f64::from(i % 3)).collect();
        assert!(assess_settle(&levels(&flat_noise)).unwrap().converged);

        // Nothing moving at all is settled whatever the trend.
        let still: Vec<f64> = (0..16).map(|i| 0.05 - 0.003 * f64::from(i)).collect();
        let verdict = assess_settle(&levels(&still)).unwrap();
        assert!(verdict.converged);
        assert!(verdict.smooth.late < SETTLED_LEVEL);
    }

    #[test]
    fn a_repeating_jitter_pattern_is_not_a_trend() {
        // The pattern repeats every 8 frames and is far from flat, but each period is the
        // same as the one before it.
        let pattern = [0.15, 0.08, 0.02, 0.057, 0.043, 0.032, 0.04, 0.053];
        for length in 16..=24 {
            let series: Vec<f64> = (0..length).map(|i| pattern[i % 8]).collect();
            let verdict = assess_settle(&levels(&series)).unwrap();
            assert!(verdict.converged, "{length} frames: {verdict:?}");
        }
    }

    /// Frame-to-frame differences of a real capture (a floor and two blocks under TAA, view
    /// and scene frozen, 16 settle frames and 2 saved images), which repeat every 8 frames:
    /// the share of pixels over the threshold in percent, then the smooth difference.
    const REAL_SERIES: [(&str, [f64; 18], [f64; 18]); 3] = [
        (
            "baseline",
            [
                0.1531, 0.0796, 0.0206, 0.057, 0.0428, 0.0319, 0.0396, 0.053, 0.0979, 0.0787,
                0.0206, 0.0558, 0.043, 0.0302, 0.0349, 0.0515, 0.1018, 0.078,
            ],
            [
                0.1372, 0.128, 0.1168, 0.1256, 0.1208, 0.1181, 0.1213, 0.1201, 0.1285, 0.126,
                0.1169, 0.1254, 0.1205, 0.1187, 0.1217, 0.1208, 0.13, 0.1264,
            ],
        ),
        (
            "same",
            [
                0.0416, 0.0519, 0.101, 0.0732, 0.0203, 0.0595, 0.0416, 0.026, 0.0395, 0.0483,
                0.1007, 0.075, 0.0214, 0.0572, 0.0408, 0.0323, 0.0362, 0.0535,
            ],
            [
                0.1207, 0.1207, 0.1289, 0.1258, 0.1168, 0.1252, 0.1206, 0.119, 0.1213, 0.1208,
                0.1299, 0.1264, 0.1155, 0.1251, 0.119, 0.1187, 0.1225, 0.123,
            ],
        ),
        (
            "ssao_off",
            [
                0.232, 0.053, 0.0172, 0.0636, 0.0426, 0.0816, 0.0466, 0.0231, 0.0567, 0.0346,
                0.022, 0.0452, 0.0445, 0.0709, 0.0362, 0.0199, 0.0576, 0.0319,
            ],
            [
                0.1316, 0.1106, 0.1037, 0.1086, 0.1075, 0.1172, 0.1084, 0.105, 0.1097, 0.1062,
                0.1048, 0.1075, 0.1058, 0.1117, 0.1073, 0.1052, 0.1103, 0.1073,
            ],
        ),
    ];

    #[test]
    fn real_jitter_noise_is_not_mistaken_for_unsettled_history() {
        // An earlier rule (last third against the third before it) called the identical
        // variant unsettled on exactly this data.
        for (name, over_pct, smooth) in REAL_SERIES {
            let series: Vec<Level> = over_pct
                .iter()
                .zip(smooth)
                .map(|(&over_pct, smooth)| Level {
                    smooth,
                    over: over_pct / 100.0,
                })
                .collect();
            let verdict = assess_settle(&series).unwrap();
            assert!(verdict.converged, "{name}: {verdict:?}");
        }
    }

    #[test]
    fn either_signal_still_falling_means_not_settled() {
        // Broad differences that have levelled off, but strong changes in few pixels that
        // are still dying away: the smooth difference alone would call this settled.
        let series: Vec<Level> = (0..18)
            .map(|i| Level {
                smooth: 0.11,
                over: 0.004 * 0.8f64.powi(i),
            })
            .collect();
        let verdict = assess_settle(&series).unwrap();
        assert!(verdict.smooth.settled);
        assert!(!verdict.over.settled);
        assert!(!verdict.converged);

        // And the other way round.
        let series: Vec<Level> = (0..18)
            .map(|i| Level {
                smooth: 4.0 * 0.8f64.powi(i),
                over: 0.0004,
            })
            .collect();
        let verdict = assess_settle(&series).unwrap();
        assert!(verdict.over.settled && !verdict.smooth.settled);
        assert!(!verdict.converged);
    }

    #[test]
    fn a_short_series_gives_no_verdict() {
        assert!(assess_settle(&[]).is_none());
        let series = |length: usize| levels(&vec![1.0; length]);
        assert!(
            assess_settle(&series(15)).is_none(),
            "under two jitter periods"
        );
        assert!(assess_settle(&series(16)).is_some());
    }

    /// A frame of a gradient, lifted everywhere by `lift` and in `patch` (x, y, width,
    /// height, extra lift). Alternate frames flip a one-level checkerboard, standing in
    /// for dither: it differs at every pixel but averages out within blocks.
    fn frame_image(frame: u64, lift: u8, patch: Option<(u32, u32, u32, u32, u8)>) -> RgbaImage {
        let mut image = RgbaImage::from_fn(16, 16, |x, y| {
            let base = (x * 8 + y * 4) as u8;
            let dither = u8::from((x + y + frame as u32).is_multiple_of(2));
            let value = base.saturating_add(lift).saturating_add(dither);
            Rgba([value, value, value, 255])
        });
        if let Some((x0, y0, w, h, add)) = patch {
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    let pixel = image.get_pixel_mut(x, y);
                    for channel in 0..3 {
                        pixel.0[channel] = pixel.0[channel].saturating_add(add);
                    }
                }
            }
        }
        image
    }

    fn options(frames: u64) -> Options {
        Options {
            cols: 2,
            rows: 2,
            threshold: 4,
            gain: 10.0,
            frames,
            settle_frames: 16,
        }
    }

    /// Feed a recorder one variant: a settle frame per entry of `settle_lifts`, then
    /// `images` saved images lifted by `image_lifts` (the last value repeats). Frame numbers
    /// start at `start`; returns the next one.
    fn record(
        recorder: &mut Recorder,
        variant: usize,
        start: u64,
        settle_lifts: &[u8],
        image_lifts: &[u8],
        patch: Option<(u32, u32, u32, u32, u8)>,
    ) -> u64 {
        let mut frame = start;
        for &lift in settle_lifts {
            recorder.push(
                variant,
                FrameKind::Settle,
                frame,
                frame_image(frame, lift, patch),
            );
            frame += 1;
        }
        for &lift in image_lifts {
            recorder.push(
                variant,
                FrameKind::Image,
                frame,
                frame_image(frame, lift, patch),
            );
            frame += 1;
        }
        frame
    }

    #[test]
    fn a_patch_stands_out_from_the_dither_and_is_located() {
        let names = vec!["baseline".to_string(), "bright".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 100, &[], &[0, 0], None);
        // Bottom-right region (8..16 x 8..16) lifted by 30 levels.
        record(&mut recorder, 1, next, &[], &[0, 0], Some((8, 8, 8, 8, 30)));
        let outcome = recorder.finish();

        assert_eq!(outcome.options["reference"], "baseline");
        assert_eq!(outcome.options["grid"], json!([2, 2]));
        let (reply, manifest) = &outcome.variants[1];
        let compare = &reply["compare"];
        assert_eq!(compare["differs"], true);
        assert_eq!(compare["regions_above_noise"], "1 of 4");
        let top = &compare["top_regions"][0];
        assert_eq!(top["cell"], json!([1, 1]));
        assert_eq!(top["rect"], json!([8, 8, 8, 8]));
        assert_eq!(top["above_noise"], true);
        assert!(top["smooth"].as_f64().unwrap() > 29.0);
        assert!(top["score"].as_f64().unwrap() > 100.0);
        // The checkerboard differs at every pixel between frames: a mean of one level...
        let noise = &compare["noise"];
        assert!(
            (noise["mean"].as_f64().unwrap() - 1.0).abs() < 1e-9,
            "{noise}"
        );
        // ...that is not a difference once smoothed.
        assert_eq!(noise["smooth"], 0.0);
        assert!(compare.get("regions").is_none(), "the reply stays compact");
        let regions = manifest["compare"]["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 4);
        assert_eq!(regions[0]["above_noise"], false);
        assert!(regions[0]["noise_smooth"].is_number());

        let reference = &outcome.variants[0].0["compare"];
        assert_eq!(reference["role"], "reference");
        assert!(reference.get("differs").is_none());
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn regions_are_listed_by_how_far_they_are_above_the_noise() {
        let names = vec!["baseline".to_string(), "two".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0, 0], None);
        // Top-left lifted by 3 (under the threshold), bottom-right by 40: the bigger one
        // is listed first.
        let image = |frame, lift| {
            let mut image = frame_image(frame, 0, Some((8, 8, 8, 8, lift)));
            for y in 0..8 {
                for x in 0..8 {
                    let pixel = image.get_pixel_mut(x, y);
                    pixel.0[0] = pixel.0[0].saturating_add(3);
                }
            }
            image
        };
        recorder.push(1, FrameKind::Image, next, image(next, 40));
        recorder.push(1, FrameKind::Image, next + 1, image(next + 1, 40));
        let outcome = recorder.finish();
        let top = outcome.variants[1].0["compare"]["top_regions"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(top.len(), 2, "only regions above the noise are listed");
        assert_eq!(top[0]["cell"], json!([1, 1]));
        assert_eq!(top[1]["cell"], json!([0, 0]));
        assert!(top[0]["score"].as_f64() > top[1]["score"].as_f64());
    }

    #[test]
    fn dither_alone_is_not_a_difference() {
        let names = vec!["baseline".to_string(), "same".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0, 0], None);
        // The last frames have opposite checkerboards: every pixel differs by one level.
        record(&mut recorder, 1, next + 1, &[], &[0, 0], None);
        let outcome = recorder.finish();
        let compare = &outcome.variants[1].0["compare"];
        assert_eq!(compare["mean"], 1.0);
        assert_eq!(compare["differs"], false);
        assert_eq!(compare["regions_above_noise"], "0 of 4");
        assert!(compare.get("top_regions").is_none());
    }

    #[test]
    fn a_faint_broad_shift_below_the_dither_is_seen() {
        let names = vec!["baseline".to_string(), "dim".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0, 0], None);
        // One level lighter everywhere, no larger than the dither itself.
        record(&mut recorder, 1, next, &[], &[1, 1], None);
        let outcome = recorder.finish();
        let compare = &outcome.variants[1].0["compare"];
        assert_eq!(compare["smooth"], 1.0);
        assert_eq!(compare["over_pct"], 0.0, "no pixel is over the threshold");
        assert_eq!(compare["differs"], true);
        assert_eq!(compare["regions_above_noise"], "4 of 4");
    }

    #[test]
    fn one_frame_per_variant_warns_that_noise_is_unknown() {
        let names = vec!["baseline".to_string(), "bright".to_string()];
        let mut recorder = Recorder::new(options(1), names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0], None);
        record(&mut recorder, 1, next, &[], &[20], None);
        let outcome = recorder.finish();
        assert!(outcome.warnings.iter().any(|w| w.contains("frames is 1")));
        let compare = &outcome.variants[1].0["compare"];
        assert_eq!(compare["noise"], Value::Null);
        assert_eq!(compare["differs"], true);
    }

    #[test]
    fn settle_frames_that_keep_changing_are_reported() {
        let names = vec!["baseline".to_string(), "taa".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 0, &[0; 16], &[0, 0], None);
        // Brightness decaying by 15% a frame, as temporal history converging would; the
        // saved images carry on down the same curve.
        let decay: Vec<u8> = (0..18)
            .map(|i| (120.0 * 0.85f64.powi(i)).round() as u8)
            .collect();
        record(&mut recorder, 1, next, &decay[..16], &decay[16..], None);
        let outcome = recorder.finish();

        let steady = &outcome.variants[0].0["settle"];
        assert_eq!(steady["converged"], true);
        assert_eq!(
            steady["smooth"]["late"], 0.0,
            "dither alone is not movement"
        );
        let settle = &outcome.variants[1].0["settle"];
        assert_eq!(settle["converged"], false, "{settle}");
        assert_eq!(
            settle["frames_compared"], 17,
            "16 settle frames + 1 saved pair"
        );
        assert!(outcome
            .warnings
            .iter()
            .any(|w| w.contains("variant taa") && w.contains("had not settled")));
        let detail = &outcome.variants[1].1["settle"];
        let trace = detail["frame_to_frame"].as_array().unwrap();
        assert_eq!(trace.len(), 17);
        assert!(trace[0].as_f64() > trace[16].as_f64(), "{trace:?}");
        assert!(settle.get("frame_to_frame").is_none());
    }

    #[test]
    fn too_few_frames_say_so_instead_of_guessing() {
        let names = vec!["baseline".to_string(), "other".to_string()];
        let options = Options {
            settle_frames: 0,
            ..options(2)
        };
        let mut recorder = Recorder::new(options, names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0, 0], None);
        record(&mut recorder, 1, next, &[], &[0, 0], None);
        let outcome = recorder.finish();
        let settle = &outcome.variants[1].0["settle"];
        assert_eq!(settle["converged"], Value::Null);
        assert_eq!(settle["frames_compared"], 1);
        let note = settle["note"].as_str().unwrap();
        assert!(note.contains("there were 1"), "{note}");
        assert!(
            note.contains("0 settle frames compared, 2 saved images"),
            "{note}"
        );
        assert!(outcome.warnings.is_empty(), "no verdict is not a warning");
    }

    #[test]
    fn history_that_has_run_its_course_is_settled() {
        let names = vec!["baseline".to_string(), "taa".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        let next = record(&mut recorder, 0, 0, &[0; 16], &[0, 0], None);
        // The same decay, but it has levelled off at a constant lift well before the end.
        let lifts: Vec<u8> = (0..18)
            .map(|i| 4 + (60.0 * 0.5f64.powi(i)).round() as u8)
            .collect();
        record(&mut recorder, 1, next, &lifts[..16], &lifts[16..], None);
        let outcome = recorder.finish();
        assert_eq!(outcome.variants[1].0["settle"]["converged"], true);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn a_first_image_far_from_the_last_means_history_was_still_converging() {
        let names = vec!["baseline".to_string(), "slow".to_string()];
        let mut recorder = Recorder::new(options(3), names, None);
        let next = record(&mut recorder, 0, 0, &[], &[0, 0, 0], None);
        // Three saved images that keep brightening, with no settle frames at all.
        record(&mut recorder, 1, next, &[], &[0, 20, 40], None);
        let outcome = recorder.finish();
        let settle = &outcome.variants[1].0["settle"];
        assert_eq!(settle["drifting"], true);
        assert_eq!(settle["first_to_last"], 40.0);
        assert!(outcome
            .warnings
            .iter()
            .any(|w| w.contains("first saved image")));
        let still = &outcome.variants[0].0["settle"];
        assert_eq!(still["drifting"], false);
    }

    #[test]
    fn skipped_frames_and_size_changes_are_reported_not_hidden() {
        let names = vec!["baseline".to_string(), "other".to_string()];
        let mut recorder = Recorder::new(options(2), names, None);
        recorder.push(0, FrameKind::Image, 10, frame_image(10, 0, None));
        // Frame 14: the engine skipped three frames.
        recorder.push(0, FrameKind::Image, 14, frame_image(14, 0, None));
        // The window was resized before the second variant.
        recorder.push(1, FrameKind::Image, 15, flat(8, 8, 0));
        recorder.push(1, FrameKind::Image, 16, flat(8, 8, 0));
        let outcome = recorder.finish();
        assert!(outcome
            .warnings
            .iter()
            .any(|w| w.contains("not consecutive")));
        assert!(outcome
            .warnings
            .iter()
            .any(|w| w.contains("differ in size")));
        assert!(outcome.variants[1].0["compare"]["error"].is_string());
    }

    #[test]
    fn difference_images_are_written_next_to_the_captures() {
        let dir = std::env::temp_dir().join(format!("visual-compare-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let names = vec!["baseline".to_string(), "bright".to_string()];
        let mut recorder = Recorder::new(options(2), names, Some(dir.clone()));
        let next = record(&mut recorder, 0, 0, &[], &[0, 0], None);
        // Frame parity matches the reference's last frame, so only the patch differs.
        record(&mut recorder, 1, next, &[], &[0, 0], Some((8, 8, 8, 8, 30)));
        let outcome = recorder.finish();

        let diff = image::open(dir.join("bright_diff.png")).unwrap().to_rgba8();
        // 30 levels at a gain of 10 saturates; untouched regions stay black.
        assert_eq!(diff.get_pixel(12, 12).0[0], 255);
        assert_eq!(diff.get_pixel(2, 2).0, [0, 0, 0, 255]);
        let noise = image::open(dir.join("bright_noise.png"))
            .unwrap()
            .to_rgba8();
        assert_eq!(
            noise.get_pixel(2, 2).0,
            [10, 10, 10, 255],
            "one dither level at gain 10"
        );
        assert!(dir.join("baseline_noise.png").is_file());
        assert!(
            !dir.join("baseline_diff.png").exists(),
            "no diff of the reference"
        );
        let path = outcome.variants[1].0["compare"]["diff_image"]
            .as_str()
            .unwrap();
        assert_eq!(path, dir.join("bright_diff.png").display().to_string());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
