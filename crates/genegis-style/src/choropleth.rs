use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ColorRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ColorRgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_hex(&self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            (self.r * 255.0) as u8,
            (self.g * 255.0) as u8,
            (self.b * 255.0) as u8
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegendItem {
    pub label: String,
    pub color: ColorRgba,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoroplethStyle {
    pub field: String,
    pub unit: String,
    pub breaks: Vec<f64>,
    pub colors: Vec<ColorRgba>,
    pub legend: Vec<LegendItem>,
}

impl ChoroplethStyle {
    pub fn equal_interval(field: impl Into<String>, unit: impl Into<String>, values: &[f64], classes: usize) -> Self {
        let field = field.into();
        let unit = unit.into();
        let (min, max) = values.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(*v), b.max(*v))
        });

        let classes = classes.max(2);
        let step = if (max - min).abs() < f64::EPSILON {
            1.0
        } else {
            (max - min) / classes as f64
        };

        let palette = default_palette(classes);
        let mut breaks = Vec::with_capacity(classes + 1);
        for i in 0..=classes {
            breaks.push(min + step * i as f64);
        }

        let mut legend = Vec::new();
        for i in 0..classes {
            let lo = breaks[i];
            let hi = breaks[i + 1];
            legend.push(LegendItem {
                label: format!("{lo:.0} – {hi:.0} {unit}"),
                color: palette[i],
            });
        }

        Self {
            field,
            unit,
            breaks,
            colors: palette,
            legend,
        }
    }

    pub fn color_for(&self, value: f64) -> ColorRgba {
        if self.colors.is_empty() {
            return ColorRgba::new(0.5, 0.5, 0.5, 1.0);
        }
        for i in 0..self.colors.len().saturating_sub(1) {
            if value <= self.breaks[i + 1] {
                return self.colors[i];
            }
        }
        *self.colors.last().unwrap()
    }
}

fn default_palette(classes: usize) -> Vec<ColorRgba> {
    let base = [
        ColorRgba::new(0.93, 0.96, 0.99, 1.0),
        ColorRgba::new(0.75, 0.85, 0.95, 1.0),
        ColorRgba::new(0.45, 0.68, 0.88, 1.0),
        ColorRgba::new(0.20, 0.45, 0.75, 1.0),
        ColorRgba::new(0.05, 0.25, 0.55, 1.0),
    ];
    base.iter().take(classes).copied().collect()
}
