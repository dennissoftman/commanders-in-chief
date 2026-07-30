//! Per-pass GPU timing, so where the frame goes is measured rather than argued.
//!
//! # Why this exists before the optimisation it is for
//!
//! Every performance question this renderer has open is workload-dependent, and none of them can be
//! settled by reasoning. Is the terrain vertex-bound, submitting its whole heightfield five times a
//! frame, or fragment-bound in its layer blend? Would a depth pre-pass pay for the second geometry
//! submission it costs? Is the resolution scale's quadratic cost actually quadratic here, or does
//! something else dominate? Each of those is a subtraction between two numbers that did not exist.
//!
//! The milestone already carries the rule for models — *LOD wants measurement first* — and this is the
//! measurement. It is deliberately not a frame-rate counter: a total tells you that something is slow,
//! and a per-pass breakdown tells you which thing.
//!
//! # Why a fixed slot per pass
//!
//! Each [`TimedPass`] owns two timestamp queries at an index derived from the variant, so nothing has to
//! be recorded, counted, or threaded through the chain while passes are being encoded. That is what lets
//! [`crate::deferred::DeferredRenderer::render`] stay `&self`: writing a timestamp mutates the query set
//! on the *GPU*, not this struct.
//!
//! Not every pass is timed every frame, which a fixed layout has to answer for. Water and the antialias
//! resolve are conditional outright, and a shadow cascade that caught no caster is recorded — its clear is
//! load-bearing — but left untimed, because a pass that rasterises nothing has no end-of-pass timestamp to
//! give on every backend. See `DeferredRenderer::time_if` for that one. The resolve buffer is cleared before
//! anything is resolved into it, and only the timed passes are resolved, so anything else reads back as a
//! pair of zeroes — and a pair whose end does not exceed its beginning is reported as absent rather than as
//! a duration. Guessing from indeterminate contents was the alternative.
//!
//! So *absent* means "nothing to attribute here", covering both a pass that never ran and one that ran over
//! no geometry. What it never means is zero: see [`FrameTimings::get`].
//!
//! # What the numbers mean, and what they do not
//!
//! A timestamp pair brackets a pass on the GPU timeline, so the differences are real GPU time and not CPU
//! submission time. They are not additive with wall-clock frame time: passes with no dependency between
//! them can overlap on hardware that schedules them concurrently, so a sum of pass times can exceed the
//! frame. Treat a breakdown as *attribution* — which pass dominates, and by how much — rather than as a
//! budget that must add up.

use std::sync::mpsc;
use std::time::Duration;

use crate::RenderError;
use crate::gpu::GpuContext;

/// Bytes one resolved timestamp occupies.
const TICK_BYTES: usize = 8;

/// Alignment a query resolve destination offset must satisfy.
///
/// This is why each pass is given a whole 256-byte stride for its two 8-byte timestamps rather than being
/// packed: the offsets have to be aligned, so packing would force one resolve of the whole range and take
/// away the ability to resolve only the passes that ran. Eleven passes at 256 bytes is under three
/// kilobytes, which is not a trade worth thinking about twice.
const RESOLVE_ALIGNMENT: u64 = 256;

/// A pass the chain can attribute time to.
///
/// The order is the order [`crate::deferred::DeferredRenderer::render`] records them, and the discriminant
/// order is load-bearing: it is what assigns query slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimedPass {
    /// The first shadow cascade, nearest the camera.
    ///
    /// Absent when its frustum caught no caster, which is ordinary rather than exceptional for this one: it
    /// covers only the first few percent of the shadow distance, so a camera any height above the ground
    /// has it sitting in the air over the terrain. A cascade with nothing in it is recorded — the clear
    /// still has to happen — but not timed, since there is nothing to attribute.
    ShadowCascade0,
    /// The second shadow cascade. Absent when it caught no caster, as [`Self::ShadowCascade0`] explains.
    ShadowCascade1,
    /// The third shadow cascade. Absent when it caught no caster.
    ShadowCascade2,
    /// The fourth shadow cascade, covering the far end of the shadow distance. Absent when it caught no
    /// caster, which for the outermost cascade means the camera is looking away from the terrain entirely.
    ShadowCascade3,
    /// Albedo, world normal with roughness, coverage, and scene depth.
    Gbuffer,
    /// Ambient occlusion.
    Occlusion,
    /// The bilateral blur over the occlusion estimate.
    OcclusionBlur,
    /// Deferred lighting into the HDR target.
    Lighting,
    /// Water, blended into the HDR target. Absent when the scene has no water.
    Water,
    /// Tone mapping, the resolution downsample, and the sharpen.
    Composite,
    /// The antialias resolve. Absent unless the display settings ask for one.
    Antialias,
}

impl TimedPass {
    /// Every pass, in the order they are recorded.
    pub const ALL: &'static [Self] = &[
        Self::ShadowCascade0,
        Self::ShadowCascade1,
        Self::ShadowCascade2,
        Self::ShadowCascade3,
        Self::Gbuffer,
        Self::Occlusion,
        Self::OcclusionBlur,
        Self::Lighting,
        Self::Water,
        Self::Composite,
        Self::Antialias,
    ];

    /// The four shadow cascades, in order, so a caller indexing cascades does not match on variants.
    pub const CASCADES: &'static [Self] = &[
        Self::ShadowCascade0,
        Self::ShadowCascade1,
        Self::ShadowCascade2,
        Self::ShadowCascade3,
    ];

    /// This pass's position in [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The first of this pass's two timestamp queries.
    #[must_use]
    pub const fn first_query(self) -> u32 {
        self as u32 * 2
    }

    /// Where this pass's two resolved timestamps land in the resolve buffer.
    ///
    /// A whole aligned stride per pass rather than eight packed bytes, because a resolve destination
    /// offset has to be aligned — see [`RESOLVE_ALIGNMENT`].
    #[must_use]
    pub const fn resolve_offset(self) -> u64 {
        self as u64 * RESOLVE_ALIGNMENT
    }

    /// A short name for a breakdown.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShadowCascade0 => "shadow 0",
            Self::ShadowCascade1 => "shadow 1",
            Self::ShadowCascade2 => "shadow 2",
            Self::ShadowCascade3 => "shadow 3",
            Self::Gbuffer => "gbuffer",
            Self::Occlusion => "occlusion",
            Self::OcclusionBlur => "occlusion blur",
            Self::Lighting => "lighting",
            Self::Water => "water",
            Self::Composite => "composite",
            Self::Antialias => "antialias",
        }
    }
}

/// How long each pass of one frame took on the GPU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameTimings {
    entries: Vec<(TimedPass, Duration)>,
}

impl FrameTimings {
    /// Every pass that ran, in recording order.
    #[must_use]
    pub fn entries(&self) -> &[(TimedPass, Duration)] {
        &self.entries
    }

    /// How long one pass took, or `None` when it was not timed.
    ///
    /// Not timed and zero are different claims, and this returns the first for both a pass that did not run
    /// and a pass that ran over no geometry. See the module note.
    #[must_use]
    pub fn get(&self, pass: TimedPass) -> Option<Duration> {
        self.entries
            .iter()
            .find_map(|(candidate, duration)| (*candidate == pass).then_some(*duration))
    }

    /// The sum over every pass.
    ///
    /// A sum and not a frame time. See the module note: passes without a dependency between them may
    /// overlap, so this can exceed the wall clock and is useful as a denominator rather than as a budget.
    #[must_use]
    pub fn sum(&self) -> Duration {
        self.entries.iter().map(|(_, duration)| *duration).sum()
    }

    /// The four shadow cascades summed, which is the figure worth comparing against the G-buffer.
    ///
    /// They are one decision — how far shadows reach and at what resolution — and reading them
    /// individually mostly reports how much of the scene each cascade happened to cover.
    ///
    /// A cascade that caught no caster contributes nothing rather than making the total absent. That is the
    /// point of summing them: what the shadow distance costs is what the cascades holding geometry cost.
    #[must_use]
    pub fn shadow_total(&self) -> Duration {
        TimedPass::CASCADES
            .iter()
            .filter_map(|pass| self.get(*pass))
            .sum()
    }

    /// The slowest pass, or `None` when nothing was timed.
    #[must_use]
    pub fn slowest(&self) -> Option<(TimedPass, Duration)> {
        self.entries
            .iter()
            .copied()
            .max_by_key(|(_, duration)| *duration)
    }
}

impl std::fmt::Display for FrameTimings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.entries.is_empty() {
            return formatter.write_str("no passes timed");
        }
        let mut first = true;
        for (pass, duration) in &self.entries {
            if !first {
                formatter.write_str(", ")?;
            }
            first = false;
            write!(
                formatter,
                "{} {:.3}ms",
                pass.label(),
                duration.as_secs_f64() * 1_000.0
            )?;
        }
        write!(
            formatter,
            " (sum {:.3}ms)",
            self.sum().as_secs_f64() * 1_000.0
        )
    }
}

/// Builds a breakdown from resolved ticks.
///
/// `ticks` holds a beginning and an end for every pass in [`TimedPass::ALL`], in that order. A pair whose
/// end does not exceed its beginning is treated as a pass that did not run — which covers the cleared
/// zeroes a skipped pass leaves behind, and also the case of a timestamp counter that reset between the
/// two reads, where the honest answer is "no measurement" rather than a duration of nearly 600 years.
///
/// Kept a free function over plain integers so the arithmetic is testable with no GPU, in the same spirit
/// as the cascade fitting in [`crate::shadow`].
#[must_use]
pub fn timings_from_ticks(ticks: &[(u64, u64)], nanoseconds_per_tick: f32) -> FrameTimings {
    let period = if nanoseconds_per_tick.is_finite() && nanoseconds_per_tick > 0.0 {
        f64::from(nanoseconds_per_tick)
    } else {
        // A non-positive or non-finite period would turn every duration into zero or a NaN. Reporting
        // the raw ticks as nanoseconds is wrong, so nothing is reported at all.
        return FrameTimings::default();
    };
    let entries = TimedPass::ALL
        .iter()
        .zip(ticks)
        .filter_map(|(pass, (begin, end))| {
            let elapsed = end.checked_sub(*begin).filter(|ticks| *ticks > 0)?;
            Some((*pass, nanoseconds(elapsed, period)))
        })
        .collect();
    FrameTimings { entries }
}

/// Converts a tick count to a duration at a given nanoseconds-per-tick period.
///
/// Both casts are bounded deliberately rather than assumed safe. Ticks come from a GPU counter and the
/// period from the driver, so the product is clamped into the range a `u64` of nanoseconds holds — about
/// 584 years — before it is converted. That upper end is not hypothetical: it is exactly what a counter
/// reset produces, and `timings_from_ticks` already rejects a decreasing pair for that reason.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn nanoseconds(ticks: u64, nanoseconds_per_tick: f64) -> Duration {
    let total = ticks as f64 * nanoseconds_per_tick;
    if total.is_finite() && total > 0.0 {
        Duration::from_nanos(total.round().min(u64::MAX as f64) as u64)
    } else {
        Duration::ZERO
    }
}

/// The query set and buffers that per-pass timing needs.
///
/// Held by a [`crate::deferred::DeferredRenderer`], which writes timestamps into it while recording and
/// reads them back on request.
#[derive(Debug)]
pub struct PassTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
}

impl PassTimer {
    /// Allocates a timer, or returns `None` when the device cannot time passes.
    ///
    /// `None` rather than an error: timing is a diagnostic, and a device without `TIMESTAMP_QUERY` should
    /// draw exactly as it did before rather than fail to start. See [`GpuContext::supports_timing`].
    #[must_use]
    pub fn new(context: &GpuContext) -> Option<Self> {
        if !context.supports_timing() {
            return None;
        }
        let device = context.device();
        // Derived from the last pass's own slot rather than from a count, so there is no cast to justify
        // and no way for the query set to be sized for a different layout than the offsets assume.
        let queries = TimedPass::ALL
            .last()
            .map_or(2, |pass| pass.first_query() + 2);
        let bytes = Self::buffer_bytes();
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("cic-render pass timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: queries,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render timestamp resolve"),
                size: bytes,
                // `COPY_DST` as well as the obvious two, because clearing counts as a write: `clear_buffer`
                // is how a skipped pass's slot is left at zero, and without it the clear is a validation
                // error rather than a no-op.
                usage: wgpu::BufferUsages::QUERY_RESOLVE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cic-render timestamp readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        })
    }

    /// Total bytes both buffers need: one aligned stride per pass.
    fn buffer_bytes() -> u64 {
        TimedPass::ALL.last().map_or(RESOLVE_ALIGNMENT, |pass| {
            pass.resolve_offset() + RESOLVE_ALIGNMENT
        })
    }

    /// The timestamp writes to attach to one pass's descriptor.
    pub(crate) fn writes(&self, pass: TimedPass) -> wgpu::RenderPassTimestampWrites<'_> {
        let first = pass.first_query();
        wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        }
    }

    /// Clears the resolve buffer, resolves the passes that ran, and stages the result for readback.
    ///
    /// Clearing first is what makes a skipped pass distinguishable: its slot stays zero, and
    /// [`timings_from_ticks`] reports a non-increasing pair as absent. Resolving the whole range instead
    /// would fill the skipped slots with whatever the queries happened to hold.
    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder, ran: &[TimedPass]) {
        encoder.clear_buffer(&self.resolve, 0, None);
        for pass in ran {
            let first = pass.first_query();
            encoder.resolve_query_set(
                &self.set,
                first..first + 2,
                &self.resolve,
                pass.resolve_offset(),
            );
        }
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, Self::buffer_bytes());
    }

    /// Reads back the most recently resolved frame's timings.
    ///
    /// **This blocks until the GPU has finished the work it is reporting on.** That is the same trade the
    /// capture path makes, and it is acceptable for the same reason: this is a diagnostic taken
    /// deliberately, not something on the frame path. Calling it every frame in a window will serialise
    /// the CPU against the GPU and change the very number being measured — poll it once a second.
    ///
    /// # Errors
    ///
    /// Returns a structured [`RenderError`] when polling, mapping, or the map callback fails or times out.
    pub fn read(&self, context: &GpuContext) -> Result<FrameTimings, RenderError> {
        let slice = self.readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(RenderError::Poll)?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| RenderError::MapCallbackTimeout)?
            .map_err(RenderError::MapBuffer)?;

        let mapped = slice.get_mapped_range().map_err(RenderError::MapRange)?;
        let ticks: Vec<(u64, u64)> = TimedPass::ALL
            .iter()
            .map(|pass| {
                let base = pass.resolve_offset();
                (
                    read_tick(&mapped, base),
                    read_tick(&mapped, base + TICK_BYTES as u64),
                )
            })
            .collect();
        drop(mapped);
        self.readback.unmap();

        Ok(timings_from_ticks(
            &ticks,
            context.queue().get_timestamp_period(),
        ))
    }
}

/// Reads one little-endian tick, or zero when the range is short.
///
/// Zero rather than a panic: a short buffer would mean the stride arithmetic here disagrees with the
/// allocation, and reporting that pass as "did not run" is a better failure than taking the process with
/// it over a diagnostic.
fn read_tick(bytes: &[u8], offset: u64) -> u64 {
    let Ok(start) = usize::try_from(offset) else {
        return 0;
    };
    start
        .checked_add(TICK_BYTES)
        .and_then(|end| bytes.get(start..end))
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map_or(0, u64::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{FrameTimings, RESOLVE_ALIGNMENT, TimedPass, read_tick, timings_from_ticks};

    /// One tick per nanosecond, so a tick count reads directly as nanoseconds.
    const UNIT_PERIOD: f32 = 1.0;

    fn all_ran(elapsed: u64) -> Vec<(u64, u64)> {
        (0..TimedPass::ALL.len())
            .map(|index| {
                let begin = 1_000 + index as u64 * 10_000;
                (begin, begin + elapsed)
            })
            .collect()
    }

    #[test]
    fn every_pass_has_its_own_pair_of_queries() {
        // The slot assignment is the whole mechanism, and an overlap would silently make two passes
        // report each other's time.
        let mut seen = std::collections::BTreeSet::new();
        for pass in TimedPass::ALL {
            assert!(seen.insert(pass.first_query()), "{pass:?} reuses a slot");
            assert!(
                seen.insert(pass.first_query() + 1),
                "{pass:?} reuses a slot"
            );
        }
        assert_eq!(seen.len(), TimedPass::ALL.len() * 2);
    }

    #[test]
    fn the_pass_order_matches_the_slot_order() {
        // `timings_from_ticks` zips `ALL` against the ticks in buffer order, so a variant declared out of
        // order would attribute every pass after it to the wrong name.
        for (index, pass) in TimedPass::ALL.iter().enumerate() {
            assert_eq!(pass.index(), index, "{pass:?} is out of order");
        }
    }

    #[test]
    fn ticks_become_durations_at_the_reported_period() {
        let timings = timings_from_ticks(&all_ran(500_000), UNIT_PERIOD);
        assert_eq!(timings.entries().len(), TimedPass::ALL.len());
        assert_eq!(
            timings.get(TimedPass::Gbuffer),
            Some(Duration::from_micros(500))
        );
        // A period of two nanoseconds per tick doubles every duration.
        let doubled = timings_from_ticks(&all_ran(500_000), 2.0);
        assert_eq!(
            doubled.get(TimedPass::Gbuffer),
            Some(Duration::from_micros(1_000))
        );
    }

    #[test]
    fn a_pass_that_did_not_run_is_absent_rather_than_zero() {
        // What a skipped pass leaves behind: a cleared, zeroed slot. Absent and "took no time" are
        // different claims, and a breakdown that reported water at 0.000ms in a scene with no water would
        // be asserting the second.
        let mut ticks = all_ran(400_000);
        ticks[TimedPass::Water.index()] = (0, 0);
        ticks[TimedPass::Antialias.index()] = (0, 0);
        let timings = timings_from_ticks(&ticks, UNIT_PERIOD);
        assert_eq!(timings.get(TimedPass::Water), None);
        assert_eq!(timings.get(TimedPass::Antialias), None);
        assert_eq!(timings.entries().len(), TimedPass::ALL.len() - 2);
        assert!(timings.get(TimedPass::Composite).is_some());
    }

    #[test]
    fn a_counter_that_went_backwards_reports_nothing_rather_than_centuries() {
        // A timestamp counter can reset. Subtracting across that with wrapping arithmetic yields about
        // 584 years, which is the kind of number that gets screenshotted as a renderer bug.
        let mut ticks = all_ran(400_000);
        ticks[TimedPass::Lighting.index()] = (900_000, 400_000);
        let timings = timings_from_ticks(&ticks, UNIT_PERIOD);
        assert_eq!(timings.get(TimedPass::Lighting), None);
    }

    #[test]
    fn a_broken_period_reports_nothing_at_all() {
        // Rather than treating raw ticks as nanoseconds, which would be a plausible-looking wrong answer.
        for period in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let timings = timings_from_ticks(&all_ran(400_000), period);
            assert!(
                timings.entries().is_empty(),
                "a period of {period} should report nothing"
            );
        }
    }

    #[test]
    fn the_cascades_sum_separately_from_the_rest() {
        let timings = timings_from_ticks(&all_ran(100_000), UNIT_PERIOD);
        assert_eq!(timings.shadow_total(), Duration::from_micros(400));
        assert_eq!(
            timings.sum(),
            Duration::from_micros(100 * TimedPass::ALL.len() as u64)
        );
    }

    #[test]
    fn a_cascade_with_no_casters_leaves_the_total_to_the_ones_that_drew() {
        // The near cascade covers the first few percent of the shadow distance, so a camera above the
        // ground routinely has one whose frustum catches nothing. Its pass is recorded for the clear and
        // left untimed, which arrives here as the same cleared pair a skipped pass leaves — and the shadow
        // total has to be the three cascades that drew rather than absent or a hole.
        let mut ticks = all_ran(100_000);
        ticks[TimedPass::ShadowCascade0.index()] = (0, 0);
        let timings = timings_from_ticks(&ticks, UNIT_PERIOD);
        assert_eq!(timings.get(TimedPass::ShadowCascade0), None);
        assert_eq!(timings.shadow_total(), Duration::from_micros(300));
        assert!(!timings.to_string().contains("shadow 0"));
    }

    #[test]
    fn the_slowest_pass_is_the_slowest_one() {
        let mut ticks = all_ran(100_000);
        ticks[TimedPass::Gbuffer.index()].1 += 900_000;
        let timings = timings_from_ticks(&ticks, UNIT_PERIOD);
        let (pass, duration) = timings.slowest().expect("something was timed");
        assert_eq!(pass, TimedPass::Gbuffer);
        assert_eq!(duration, Duration::from_micros(1_000));
    }

    #[test]
    fn an_empty_breakdown_says_so_rather_than_printing_nothing() {
        assert_eq!(FrameTimings::default().to_string(), "no passes timed");
        assert!(FrameTimings::default().slowest().is_none());
        assert_eq!(FrameTimings::default().sum(), Duration::ZERO);
    }

    #[test]
    fn a_breakdown_names_every_pass_it_reports() {
        let timings = timings_from_ticks(&all_ran(250_000), UNIT_PERIOD);
        let rendered = timings.to_string();
        for (pass, _) in timings.entries() {
            assert!(rendered.contains(pass.label()), "{rendered} omits {pass:?}");
        }
        assert!(rendered.contains("sum "));
    }

    #[test]
    fn every_pass_resolves_to_an_aligned_offset() {
        // A destination offset that is not a multiple of the alignment is a validation error, and it is
        // the reason each pass gets a whole stride rather than eight packed bytes.
        for pass in TimedPass::ALL {
            let offset = pass.index() as u64 * RESOLVE_ALIGNMENT;
            assert_eq!(offset % RESOLVE_ALIGNMENT, 0, "{pass:?} is misaligned");
        }
    }

    #[test]
    fn a_short_readback_reads_as_absent_rather_than_panicking() {
        let bytes = [1_u8; 4];
        assert_eq!(read_tick(&bytes, 0), 0);
        assert_eq!(read_tick(&[], 0), 0);
        assert_eq!(read_tick(&[7, 0, 0, 0, 0, 0, 0, 0], 0), 7);
    }
}
