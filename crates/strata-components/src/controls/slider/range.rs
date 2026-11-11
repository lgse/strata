//! A numeric range with a step grid and a page: the model both of C20a's controls step through.

/// A closed range `min..=max` with a step grid anchored at `min`, a page, and the decimals a
/// value is kept to.
///
/// The grid's points are `min + k * step`, plus `max` when the range isn't a whole number of
/// steps. Keys move along the grid; a value between two points (a drag, a wheel, a value the
/// host set) moves first to the next point in the direction of travel, as Strata's thumbnail
/// ladder does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    min: f64,
    max: f64,
    step: f64,
    page: f64,
    decimals: u8,
}

/// Values closer than this to a grid point are on it: float noise from repeated steps.
const EPSILON: f64 = 1e-9;

impl Range {
    /// `min..=max` in steps of `step`, a page of ten steps (Rune's `on_step` page), and as
    /// many decimals as `step` has (GTK's spin button rule): 1 keeps none, 0.1 keeps one.
    ///
    /// # Panics
    ///
    /// When `min` isn't below `max`, `step` isn't positive, or any of them isn't finite.
    pub fn new(min: f64, max: f64, step: f64) -> Range {
        assert!(
            min.is_finite() && max.is_finite() && step.is_finite(),
            "a range is finite"
        );
        assert!(min < max, "a range's min is below its max: {min}..={max}");
        assert!(step > 0.0, "a range's step is positive: {step}");
        Range {
            min,
            max,
            step,
            page: step * 10.0,
            decimals: decimals_of(step),
        }
    }

    /// Page Up and Page Down move `page` (rounded to whole steps, at least one): Strata's
    /// density sliders page 4 px.
    pub fn page(mut self, page: f64) -> Range {
        assert!(page.is_finite() && page > 0.0, "a page is positive: {page}");
        self.page = page;
        self
    }

    /// Keep values to `decimals` places, in place of the step's own.
    pub fn decimals(mut self, decimals: u8) -> Range {
        self.decimals = decimals.min(9);
        self
    }

    pub fn min(&self) -> f64 {
        self.min
    }

    pub fn max(&self) -> f64 {
        self.max
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    /// The page, in whole steps.
    pub fn page_steps(&self) -> i32 {
        ((self.page / self.step).round() as i32).max(1)
    }

    pub fn decimal_places(&self) -> u8 {
        self.decimals
    }

    /// `v` inside the range and rounded to the decimals. Not a number is the minimum.
    pub fn clamp(&self, v: f64) -> f64 {
        if v.is_nan() {
            return self.min;
        }
        self.round(v.clamp(self.min, self.max))
    }

    /// `v` rounded to the decimals kept.
    pub fn round(&self, v: f64) -> f64 {
        let f = 10f64.powi(i32::from(self.decimals));
        (v * f).round() / f
    }

    /// The grid point nearest `v` (or `max`, where it is nearer than the last point).
    pub fn snap_to_grid(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        let k = ((v - self.min) / self.step).round();
        let on = self.min + k * self.step;
        let on = if on > self.max + EPSILON || (self.max - v).abs() < (on - v).abs() {
            self.max
        } else {
            on
        };
        self.clamp(on)
    }

    /// `v` moved `steps` grid points: up for positive, down for negative. From between two
    /// points the first step lands on the next one in that direction. The ends hold.
    pub fn stepped(&self, v: f64, steps: i32) -> f64 {
        let v = self.clamp(v);
        if steps == 0 {
            return v;
        }
        let at = (v - self.min) / self.step;
        let base = if steps > 0 {
            (at + EPSILON).floor()
        } else {
            (at - EPSILON).ceil()
        };
        let target = self.min + (base + f64::from(steps)) * self.step;
        self.clamp(target)
    }

    /// Where `v` sits between the ends, 0 to 1.
    pub fn fraction(&self, v: f64) -> f64 {
        ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    /// The value at fraction `f` of the way from `min` to `max`, unrounded.
    pub fn at(&self, f: f64) -> f64 {
        self.min + f.clamp(0.0, 1.0) * (self.max - self.min)
    }

    /// `v` as text, to the decimals kept.
    pub fn format(&self, v: f64) -> String {
        format!("{:.*}", usize::from(self.decimals), self.clamp(v))
    }
}

/// The decimals `step` has, up to six: 1 has none, 0.25 two.
fn decimals_of(step: f64) -> u8 {
    for d in 0..6u8 {
        let f = 10f64.powi(i32::from(d));
        if ((step * f).round() - step * f).abs() < 1e-6 {
            return d;
        }
    }
    6
}
