use super::analysis::Spectrum;
use super::*;
const WIDTH: usize = 900;
const HEIGHT: usize = 448;
const INK: [u8; 3] = [227, 234, 243];
struct Canvas(Vec<u8>);
impl Canvas {
    fn new() -> Self {
        Self(vec![18; WIDTH * HEIGHT * 3])
    }
    fn pixel(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            let i = (y as usize * WIDTH + x as usize) * 3;
            self.0[i..i + 3].copy_from_slice(&color);
        }
    }
    fn text(&mut self, text: &str, mut x: i32, y: i32) {
        for c in text.chars() {
            let rows = match c {
                '0' => [7, 5, 5, 5, 7],
                '1' => [2, 6, 2, 2, 7],
                '2' => [7, 1, 7, 4, 7],
                '3' => [7, 1, 7, 1, 7],
                '4' => [5, 5, 7, 1, 1],
                '5' => [7, 4, 7, 1, 7],
                '6' => [7, 4, 7, 5, 7],
                '7' => [7, 1, 2, 2, 2],
                '8' => [7, 5, 7, 5, 7],
                '9' => [7, 5, 7, 1, 7],
                '.' => [0, 0, 0, 0, 2],
                '-' => [0, 0, 7, 0, 0],
                'H' => [5, 5, 7, 5, 5],
                'Z' => [7, 1, 2, 4, 7],
                'S' => [7, 4, 7, 1, 7],
                'D' => [6, 5, 5, 5, 6],
                'B' => [6, 5, 6, 5, 6],
                'F' => [7, 4, 6, 4, 4],
                'L' => [4, 4, 4, 4, 7],
                'R' => [6, 5, 6, 5, 5],
                'M' => [5, 7, 7, 5, 5],
                _ => [0; 5],
            };
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) != 0 {
                        for dy in 0..2 {
                            for dx in 0..2 {
                                self.pixel(x + col * 2 + dx, y + row as i32 * 2 + dy, INK);
                            }
                        }
                    }
                }
            }
            x += 8;
        }
    }
    fn finish(mut self, mut meta: Value) -> Result<(Value, Vec<u8>)> {
        let start = num(&meta, "start");
        let end = num(&meta, "end");
        for tick in 0..=8 {
            let f = tick as f64 / 8.;
            let x = 70 + (f * 799.).round() as i32;
            let value = start + (end - start) * f;
            let label = if end - start < 0.1 {
                format!("{value:.4}")
            } else {
                format!("{value:.2}")
            };
            self.text(
                &label,
                (898 - label.len() as i32 * 8).min(x - label.len() as i32 * 4),
                390,
            );
            for y in 380..385 {
                self.pixel(x, y, INK)
            }
        }
        self.text("S", 470, 416);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, WIDTH as u32, HEIGHT as u32);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&self.0)?;
        }
        meta["width"] = json!(WIDTH);
        meta["height"] = json!(HEIGHT);
        Ok((meta, png))
    }
}
pub(super) fn waveform(
    a: &Audio,
    meta: Value,
    first: usize,
    last: usize,
) -> Result<(Value, Vec<u8>)> {
    let mut canvas = Canvas::new();
    for (ch, samples) in a.channels.iter().enumerate() {
        let height = 360 / a.channels.len() as i32;
        let top = 20 + ch as i32 * height;
        let bottom = top + height - 4;
        let zero = (top + bottom + 1) / 2;
        let y = |x: f32| {
            (zero as f64 - f64::from(x.clamp(-1., 1.)) * (bottom - top) as f64 / 2.).round() as i32
        };
        canvas.text(
            if a.channels.len() == 1 {
                "M"
            } else if ch == 0 {
                "L"
            } else {
                "R"
            },
            13,
            zero - 4,
        );
        canvas.text("1", 42, top);
        canvas.text("0", 42, zero - 4);
        canvas.text("-1", 34, bottom - 9);
        for col in 0..800 {
            let i = first + (last - first) * col / 800;
            let next = (first + (last - first) * (col + 1) / 800).max(i + 1);
            let slice = &samples[i..next];
            let min = slice.iter().copied().fold(f32::INFINITY, f32::min);
            let max = slice.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let x = 70 + col as i32;
            canvas.pixel(x, zero, [65, 79, 94]);
            let color = if min < -1. || max > 1. {
                [255, 140, 83]
            } else {
                [112, 222, 194]
            };
            for row in y(max)..=y(min) {
                canvas.pixel(x, row, color);
            }
        }
    }
    canvas.finish(meta)
}
fn color(f: f64) -> [u8; 3] {
    let palette = [
        [8., 10., 25.],
        [44., 33., 105.],
        [130., 44., 118.],
        [215., 77., 74.],
        [249., 161., 60.],
        [253., 247., 171.],
    ];
    let p = f.clamp(0., 1.) * 5.;
    let i = (p as usize).min(4);
    std::array::from_fn(|ch| {
        (palette[i][ch] + (palette[i + 1][ch] - palette[i][ch]) * (p - i as f64)).round() as u8
    })
}
pub(super) fn spectrogram(
    a: &Audio,
    c: &Value,
    mut meta: Value,
    size: usize,
) -> Result<(Value, Vec<u8>)> {
    let scale = option_str(c, "scale", "log")?;
    if !matches!(scale, "log" | "linear") {
        return Err("scale must be log or linear".into());
    }
    let log = scale == "log";
    let min = number(
        c,
        "minHz",
        if log { 30. } else { 0. },
        if log { 1. } else { 0. },
        a.rate / 2.,
    )?;
    let max = number(c, "maxHz", 16000f64.min(a.rate / 2.), min, a.rate / 2.)?;
    if max <= min {
        return Err("maxHz must be greater than minHz".into());
    }
    let floor = number(c, "floorDb", -85., -150., -1.)?;
    let ceiling = number(c, "ceilingDb", 0., floor + 1., 48.)?;
    let freq = |f: f64| {
        if log {
            min * (max / min).powf(f)
        } else {
            min + f * (max - min)
        }
    };
    let first = (num(&meta, "start") * a.rate).round() as usize;
    let span = ((num(&meta, "end") * a.rate).round() as usize - first).max(1);
    let subs = ((span as f64 / 800. / (size as f64 / 4.)).ceil() as usize).max(1);
    let bins: Vec<_> = (0..360)
        .map(|r| {
            let lo = freq(r as f64 / 360.) * size as f64 / a.rate;
            let hi = freq((r + 1) as f64 / 360.) * size as f64 / a.rate;
            (lo.ceil() as usize, hi.floor() as usize, (lo + hi) / 2.)
        })
        .collect();
    let mut canvas = Canvas::new();
    let mut spectrum = Spectrum::new(size, false, a.channels.len());
    for col in 0..800 {
        let mut levels = [0f32; 360];
        for sub in 0..subs {
            let position = (col as f64 + (sub as f64 + 0.5) / subs as f64) / 800.;
            let powers =
                spectrum.powers(a, first + ((span as f64 * position) as usize).min(span - 1));
            for (r, &(lo, hi, center)) in bins.iter().enumerate() {
                let power = if lo <= hi {
                    powers[lo..=hi.min(size / 2)]
                        .iter()
                        .copied()
                        .fold(0., f64::max)
                } else {
                    let i = center as usize;
                    powers[i] * (1. - center.fract())
                        + powers[(i + 1).min(size / 2)] * center.fract()
                };
                levels[r] = levels[r].max(power as f32);
            }
        }
        for (r, p) in levels.iter().enumerate() {
            canvas.pixel(
                70 + col,
                379 - r as i32,
                color((10. * f64::from(*p).log10() - floor) / (ceiling - floor)),
            )
        }
    }
    canvas.text("HZ", 14, 3);
    for tick in 0..=6 {
        let f = tick as f64 / 6.;
        let y = 20 + ((1. - f) * 359.).round() as i32;
        canvas.text(&format!("{}", freq(f).round()), 14, y - 4);
        for x in 65..70 {
            canvas.pixel(x, y, INK)
        }
    }
    for col in 0..180 {
        for y in 416..424 {
            canvas.pixel(114 + col, y, color(col as f64 / 179.))
        }
    }
    canvas.text(&format!("{floor}"), 75, 415);
    canvas.text(&format!("{ceiling} DBFS"), 300, 415);
    meta["scale"] = json!(scale);
    meta["minHz"] = json!(min);
    meta["maxHz"] = json!(max);
    meta["floorDb"] = json!(floor);
    meta["ceilingDb"] = json!(ceiling);
    canvas.finish(meta)
}
