//! GDI+ drawing into a layered window's pixels, for the reading pill. Ported
//! from Inlay's Windows theme. Colours are 0xAARRGGBB; sizes are device pixels.
//! Text goes through GDI+ too, so it keeps the pixels' alpha.

use std::ptr::null_mut;
use std::sync::OnceLock;

use windows_sys::Win32::Graphics::GdiPlus::*;

/// GDI+'s premultiplied 32-bit format, which layered windows expect.
const PIXEL_FORMAT_PARGB: i32 = 0xE200B;

/// Starts GDI+ for the life of the process. Safe to call more than once.
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0;
        let mut output = GdiplusStartupOutput::default();
        unsafe { GdiplusStartup(&mut token, &input, &mut output) };
    });
}

/// `rgb` (0xRRGGBB) at `opacity` from 0 to 1, as 0xAARRGGBB.
pub fn alpha(rgb: u32, opacity: f32) -> u32 {
    (((opacity.clamp(0.0, 1.0) * 255.0).round() as u32) << 24) | (rgb & 0xFF_FFFF)
}

/// Mixes two 0xAARRGGBB colours channel by channel.
pub fn mix(from: u32, to: u32, t: f32) -> u32 {
    (0..4).fold(0, |colour, channel| {
        let shift = channel * 8;
        let (a, b) = ((from >> shift) & 0xFF, (to >> shift) & 0xFF);
        let value = (a as f32 + (b as f32 - a as f32) * t).round() as u32;
        colour | (value.min(255) << shift)
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn inflate(self, by: f32) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + by * 2.0,
            h: self.h + by * 2.0,
        }
    }
}

pub struct Canvas {
    graphics: *mut GpGraphics,
    bitmap: *mut GpBitmap,
}

impl Canvas {
    /// Draws into the top-down 32-bit premultiplied `pixels`, `width` by `height`.
    pub fn over_pixels(pixels: *mut u8, width: i32, height: i32) -> Canvas {
        start();
        let mut bitmap = null_mut();
        let mut graphics = null_mut();
        unsafe {
            GdipCreateBitmapFromScan0(
                width,
                height,
                width * 4,
                PIXEL_FORMAT_PARGB,
                pixels,
                &mut bitmap,
            );
            GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics);
            GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
            GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        }
        Canvas { graphics, bitmap }
    }

    fn rounded(area: Rect, radius: f32) -> *mut GpPath {
        let diameter = (radius * 2.0).min(area.w).min(area.h).max(0.1);
        let (right, bottom) = (area.x + area.w - diameter, area.y + area.h - diameter);
        let mut path = null_mut();
        unsafe {
            GdipCreatePath(FillModeAlternate, &mut path);
            GdipAddPathArc(path, area.x, area.y, diameter, diameter, 180.0, 90.0);
            GdipAddPathArc(path, right, area.y, diameter, diameter, 270.0, 90.0);
            GdipAddPathArc(path, right, bottom, diameter, diameter, 0.0, 90.0);
            GdipAddPathArc(path, area.x, bottom, diameter, diameter, 90.0, 90.0);
            GdipClosePathFigure(path);
        }
        path
    }

    fn with_brush(&self, colour: u32, draw: impl FnOnce(*mut GpBrush)) {
        unsafe {
            let mut brush = null_mut();
            GdipCreateSolidFill(colour, &mut brush);
            draw(brush.cast());
            GdipDeleteBrush(brush.cast());
        }
    }

    fn with_pen(&self, colour: u32, width: f32, draw: impl FnOnce(*mut GpPen)) {
        unsafe {
            let mut pen = null_mut();
            GdipCreatePen1(colour, width, UnitPixel, &mut pen);
            GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
            GdipSetPenLineJoin(pen, LineJoinRound);
            draw(pen);
            GdipDeletePen(pen);
        }
    }

    /// A filled rounded rectangle.
    pub fn round(&self, area: Rect, radius: f32, colour: u32) {
        let path = Self::rounded(area, radius);
        self.with_brush(colour, |brush| unsafe {
            GdipFillPath(self.graphics, brush, path);
        });
        unsafe { GdipDeletePath(path) };
    }

    /// A rounded outline `width` wide, drawn inside `area`.
    pub fn outline(&self, area: Rect, radius: f32, colour: u32, width: f32) {
        let path = Self::rounded(area.inflate(-width / 2.0), radius - width / 2.0);
        self.with_pen(colour, width, |pen| unsafe {
            GdipDrawPath(self.graphics, pen, path);
        });
        unsafe { GdipDeletePath(path) };
    }

    /// Limits later drawing to a rounded rectangle.
    pub fn clip_round(&self, area: Rect, radius: f32) {
        let path = Self::rounded(area, radius);
        unsafe {
            GdipSetClipPath(self.graphics, path, CombineModeReplace);
            GdipDeletePath(path);
        }
    }

    pub fn reset_clip(&self) {
        unsafe { GdipResetClip(self.graphics) };
    }

    pub fn circle(&self, (x, y): (f32, f32), radius: f32, colour: u32) {
        self.with_brush(colour, |brush| unsafe {
            GdipFillEllipse(
                self.graphics,
                brush,
                x - radius,
                y - radius,
                radius * 2.0,
                radius * 2.0,
            );
        });
    }

    /// Part of a circle's outline. Angles are degrees clockwise from three o'clock.
    pub fn arc(
        &self,
        (x, y): (f32, f32),
        radius: f32,
        (start, sweep): (f32, f32),
        colour: u32,
        width: f32,
    ) {
        self.with_pen(colour, width, |pen| unsafe {
            GdipDrawArc(
                self.graphics,
                pen,
                x - radius,
                y - radius,
                radius * 2.0,
                radius * 2.0,
                start,
                sweep,
            );
        });
    }

    /// Joins the points with a round-capped line.
    pub fn lines(&self, points: &[(f32, f32)], colour: u32, width: f32) {
        self.with_pen(colour, width, |pen| unsafe {
            for pair in points.windows(2) {
                GdipDrawLine(
                    self.graphics,
                    pen,
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1,
                );
            }
        });
    }

    pub fn polygon(&self, points: &[(f32, f32)], colour: u32) {
        let points: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
        self.with_brush(colour, |brush| unsafe {
            GdipFillPolygon(
                self.graphics,
                brush,
                points.as_ptr(),
                points.len() as i32,
                FillModeAlternate,
            );
        });
    }

    fn with_text(
        &self,
        text: &str,
        size: f32,
        bold: bool,
        centred: bool,
        draw: impl FnOnce(&[u16], *mut GpFont, *mut GpStringFormat),
    ) {
        let units: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let mut font = null_mut();
            let style = if bold {
                FontStyleBold
            } else {
                FontStyleRegular
            };
            GdipCreateFont(family(), size, style, UnitPixel, &mut font);
            let mut format = null_mut();
            GdipCreateStringFormat(StringFormatFlagsNoWrap, 0, &mut format);
            let align = if centred {
                StringAlignmentCenter
            } else {
                StringAlignmentNear
            };
            GdipSetStringFormatAlign(format, align);
            GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
            draw(&units, font, format);
            GdipDeleteStringFormat(format);
            GdipDeleteFont(font);
        }
    }

    /// One line of Segoe UI `size` pixels tall, vertically centred in `area`.
    pub fn text(&self, text: &str, area: Rect, size: f32, bold: bool, centred: bool, colour: u32) {
        let layout = RectF {
            X: area.x,
            Y: area.y,
            Width: area.w,
            Height: area.h,
        };
        self.with_text(text, size, bold, centred, |units, font, format| {
            self.with_brush(colour, |brush| unsafe {
                GdipDrawString(
                    self.graphics,
                    units.as_ptr(),
                    units.len() as i32,
                    font,
                    &layout,
                    format,
                    brush,
                );
            });
        });
    }

    /// How wide `text` draws on one line.
    pub fn text_width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let mut bounds = RectF::default();
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 10_000.0,
            Height: 1_000.0,
        };
        self.with_text(text, size, bold, false, |units, font, format| unsafe {
            GdipMeasureString(
                self.graphics,
                units.as_ptr(),
                units.len() as i32,
                font,
                &layout,
                format,
                &mut bounds,
                null_mut(),
                null_mut(),
            );
        });
        bounds.Width
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            GdipFlush(self.graphics, FlushIntentionSync);
            GdipDeleteGraphics(self.graphics);
            GdipDisposeImage(self.bitmap.cast());
        }
    }
}

/// Segoe UI, created once.
fn family() -> *mut GpFontFamily {
    static FAMILY: OnceLock<usize> = OnceLock::new();
    *FAMILY.get_or_init(|| {
        start();
        let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let mut family = null_mut();
        unsafe { GdipCreateFontFamilyFromName(name.as_ptr(), null_mut(), &mut family) };
        family as usize
    }) as *mut GpFontFamily
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_mix_and_fade() {
        assert_eq!(alpha(0xA7D6F5, 1.0), 0xFFA7_D6F5);
        assert_eq!(alpha(0xFFA7_D6F5, 0.0), 0x00A7_D6F5);
        assert_eq!(mix(0xFF00_0000, 0xFF20_4060, 0.5), 0xFF10_2030);
    }

    #[test]
    fn draws_into_pixels_with_alpha() {
        let (w, h) = (20, 10);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        {
            let canvas = Canvas::over_pixels(pixels.as_mut_ptr(), w, h);
            let area = Rect {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            };
            canvas.round(area, 5.0, 0xFF05_080B);
        }
        // The centre is the opaque fill; the rounded corner stays clear.
        let at = |x: i32, y: i32| (y * w + x) as usize * 4;
        assert_eq!(&pixels[at(10, 5)..at(10, 5) + 4], [0x0B, 0x08, 0x05, 0xFF]);
        assert_eq!(pixels[at(0, 0) + 3], 0);
    }
}
