// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Canvas fonts and the font list (docs/fonts.md) on `OH_Drawing`'s font manager
//! (`ohos_sys::drawing`).
//!
//! The process-wide font manager is the SYSTEM font collection. It does not see what the
//! ability registered through ArkTS `font.registerFont` (that feeds the text engine labels draw
//! with, not native drawing), so the bundled families the canvas may be asked for are registered
//! separately, from their bytes ([`register_canvas_font`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;

use ohos_sys::drawing::font::{
    OH_Drawing_Font_Metrics, OH_Drawing_FontCreate, OH_Drawing_FontDestroy,
    OH_Drawing_FontGetMetrics, OH_Drawing_FontMeasureText, OH_Drawing_FontSetFakeBoldText,
    OH_Drawing_FontSetTextSize, OH_Drawing_FontSetTextSkewX, OH_Drawing_FontSetTypeface,
};
use ohos_sys::drawing::font_mgr::{
    OH_Drawing_FontMgrCreate, OH_Drawing_FontMgrCreateFontStyleSet,
    OH_Drawing_FontMgrDestroyFamilyName, OH_Drawing_FontMgrDestroyFontStyleSet,
    OH_Drawing_FontMgrGetFamilyCount, OH_Drawing_FontMgrGetFamilyName,
    OH_Drawing_FontMgrMatchFamilyStyle, OH_Drawing_FontStyleSetCount,
    OH_Drawing_FontStyleSetFreeStyleName, OH_Drawing_FontStyleSetGetStyle,
};
use ohos_sys::drawing::memory_stream::OH_Drawing_MemoryStreamCreate;
use ohos_sys::drawing::text_typography::{
    OH_Drawing_FontStyle, OH_Drawing_FontStyleStruct, OH_Drawing_FontWeight, OH_Drawing_FontWidth,
};
use ohos_sys::drawing::typeface::{
    OH_Drawing_TypefaceCreateFromStream, OH_Drawing_TypefaceDestroy,
};
use ohos_sys::drawing::types::{
    OH_Drawing_Font, OH_Drawing_FontMgr, OH_Drawing_TextEncoding, OH_Drawing_Typeface,
};

/// The family/weight/slant a canvas text record asks for.
#[derive(Clone, Default)]
pub struct FontReq {
    /// CSS 100 … 900, 0 = default.
    pub weight: i32,
    pub italic: bool,
    pub family: String,
}

thread_local! {
    static MANAGER: Cell<*mut OH_Drawing_FontMgr> = const { Cell::new(ptr::null_mut()) };
    /// The bundled families, by lower-cased name, as typefaces built from the font files'
    /// bytes; kept for the process, shared by every font that uses them.
    static CANVAS_FONTS: RefCell<HashMap<String, *mut OH_Drawing_Typeface>> =
        RefCell::new(HashMap::new());
}

fn manager() -> *mut OH_Drawing_FontMgr {
    MANAGER.with(|m| {
        if m.get().is_null() {
            // SAFETY: creates the process's manager; kept for its lifetime.
            m.set(unsafe { OH_Drawing_FontMgrCreate() });
        }
        m.get()
    })
}

/// A font for canvas text: the family's face nearest the requested weight and slant, or the
/// default face with a synthesized bold and slant. The returned typeface, when any, is the
/// font's match to destroy after it.
pub fn make_font(size: f32, req: &FontReq) -> (*mut OH_Drawing_Font, *mut OH_Drawing_Typeface) {
    // SAFETY: OH_Drawing objects created here and released by the caller through
    // `destroy_font`; the manager and bundled typefaces live for the process.
    unsafe {
        let font = OH_Drawing_FontCreate();
        OH_Drawing_FontSetTextSize(font, size);
        let weight = if req.weight > 0 { req.weight } else { 400 };
        let style = OH_Drawing_FontStyleStruct {
            // FONT_WEIGHT_100 == 0
            weight: OH_Drawing_FontWeight((weight / 100 - 1).clamp(0, 8) as _),
            width: OH_Drawing_FontWidth::FONT_WIDTH_NORMAL,
            slant: if req.italic {
                OH_Drawing_FontStyle::FONT_STYLE_ITALIC
            } else {
                OH_Drawing_FontStyle::FONT_STYLE_NORMAL
            },
        };
        // A bundled family first: its typeface is the process's, shared, never destroyed
        // here, and one face, so weight and slant are synthesized on it.
        if !req.family.is_empty() {
            let bundled =
                CANVAS_FONTS.with(|f| f.borrow().get(&req.family.to_ascii_lowercase()).copied());
            if let Some(tf) = bundled.filter(|tf| !tf.is_null()) {
                OH_Drawing_FontSetTypeface(font, tf);
                OH_Drawing_FontSetFakeBoldText(font, weight >= 600);
                OH_Drawing_FontSetTextSkewX(font, if req.italic { -0.25 } else { 0.0 });
                return (font, ptr::null_mut());
            }
        }
        // Then a NAMED system family through the manager (a null family name is not
        // something every OH_Drawing release matches); the default face keeps the font's own.
        let mut tf = ptr::null_mut();
        if !req.family.is_empty() {
            let family = crate::node::cstr(&req.family);
            tf = OH_Drawing_FontMgrMatchFamilyStyle(manager(), family.as_ptr(), style);
        }
        if !tf.is_null() {
            OH_Drawing_FontSetTypeface(font, tf);
        } else {
            OH_Drawing_FontSetFakeBoldText(font, weight >= 600);
            OH_Drawing_FontSetTextSkewX(font, if req.italic { -0.25 } else { 0.0 });
        }
        (font, tf)
    }
}

pub fn destroy_font(font: *mut OH_Drawing_Font, typeface: *mut OH_Drawing_Typeface) {
    // SAFETY: the pair `make_font` returned.
    unsafe {
        OH_Drawing_FontDestroy(font);
        if !typeface.is_null() {
            OH_Drawing_TypefaceDestroy(typeface);
        }
    }
}

/// The font's metrics: Skia-shaped, ascent NEGATIVE (above the baseline), descent positive.
pub fn metrics(font: *mut OH_Drawing_Font) -> OH_Drawing_Font_Metrics {
    // SAFETY: a zeroed metrics struct the call fills for a live font.
    unsafe {
        let mut m: OH_Drawing_Font_Metrics = std::mem::zeroed();
        OH_Drawing_FontGetMetrics(font, &mut m);
        m
    }
}

/// The advance width of `text` in `font`, or a guess from the size when measuring fails.
pub fn text_width(font: *mut OH_Drawing_Font, text: &str, size: f32) -> f32 {
    let mut w = 0.0f32;
    // SAFETY: the text's bytes are valid for the call; no bounds are requested.
    let ok = unsafe {
        OH_Drawing_FontMeasureText(
            font,
            text.as_ptr().cast(),
            text.len(),
            OH_Drawing_TextEncoding::TEXT_ENCODING_UTF8,
            ptr::null_mut(),
            &mut w,
        )
    };
    if ok.is_ok() {
        w
    } else {
        text.len() as f32 * size * 0.56
    }
}

/// Append `s` with Day's list separators replaced, so a family or style name can never split
/// the record it sits in.
fn list_append(out: &mut String, s: &str) {
    out.extend(s.chars().map(|c| match c {
        '\u{1f}' | '\u{1e}' => ' ',
        c => c,
    }));
}

/// Every family the font manager knows, with its faces, in Day's font-list text (U+001E
/// between families, U+001F between fields: family, then (style name, CSS weight, italic 0/1)
/// per face); `day_spec::parse_font_list` reads it.
pub fn families_text() -> Option<String> {
    let mgr = manager();
    if mgr.is_null() {
        return None;
    }
    let mut text = String::new();
    // SAFETY: the manager's own strings and style sets, each released after reading.
    unsafe {
        let count = OH_Drawing_FontMgrGetFamilyCount(mgr);
        for i in 0..count {
            let family = OH_Drawing_FontMgrGetFamilyName(mgr, i);
            if family.is_null() {
                continue;
            }
            let name = CStr::from_ptr(family).to_string_lossy().into_owned();
            OH_Drawing_FontMgrDestroyFamilyName(family);
            if name.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\u{1e}');
            }
            list_append(&mut text, &name);
            let set = OH_Drawing_FontMgrCreateFontStyleSet(mgr, i);
            if set.is_null() {
                continue;
            }
            let faces = OH_Drawing_FontStyleSetCount(set);
            for j in 0..faces {
                let mut style_name = ptr::null_mut();
                let st = OH_Drawing_FontStyleSetGetStyle(set, j, &mut style_name);
                text.push('\u{1f}');
                if !style_name.is_null() {
                    let s = CStr::from_ptr(style_name).to_string_lossy().into_owned();
                    list_append(&mut text, &s);
                    OH_Drawing_FontStyleSetFreeStyleName(&mut style_name);
                }
                text.push('\u{1f}');
                // FONT_WEIGHT_100 == 0
                text.push_str(&((st.weight.0 as i32 + 1) * 100).to_string());
                text.push('\u{1f}');
                text.push(if st.slant != OH_Drawing_FontStyle::FONT_STYLE_NORMAL {
                    '1'
                } else {
                    '0'
                });
            }
            OH_Drawing_FontMgrDestroyFontStyleSet(set);
        }
    }
    Some(text)
}

/// Register a bundled font for canvas text (docs/fonts.md): `data` is the font file's bytes
/// (copied), `family` the name the app draws with. The typeface lives for the process; a
/// family registered twice takes the later file. True when the bytes made a typeface.
pub fn register_canvas_font(family: &str, data: &[u8]) -> bool {
    if family.is_empty() || data.is_empty() {
        return false;
    }
    // SAFETY: the stream copies the bytes; ownership of the stream passes to the typeface
    // (the header's contract), so it is not destroyed here whether or not the bytes parsed.
    let tf = unsafe {
        let stream = OH_Drawing_MemoryStreamCreate(data.as_ptr().cast(), data.len(), true);
        if stream.is_null() {
            return false;
        }
        OH_Drawing_TypefaceCreateFromStream(stream, 0)
    };
    if tf.is_null() {
        return false;
    }
    CANVAS_FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(old) = f.insert(family.to_ascii_lowercase(), tf)
            && !old.is_null()
        {
            // SAFETY: a typeface this module created and no font references any more.
            unsafe { OH_Drawing_TypefaceDestroy(old) };
        }
    });
    true
}

/// Measure one line of canvas text with the font [`make_font`] resolves: the eight slots
/// `day_spec::TextMetrics::from_slots` reads (advance width, line height, ascent, cap height,
/// then the ink box, which is the whole line box here: OH_Drawing's C surface has no
/// tight-bounds call, and the superset is what the contract allows, docs/fonts.md).
pub fn measure_text(text: &str, size: f64, weight: i32, italic: bool, family: &str) -> [f64; 8] {
    let req = FontReq {
        weight,
        italic,
        family: family.to_owned(),
    };
    let (font, tf) = make_font(size as f32, &req);
    let w = f64::from(text_width(font, text, size as f32));
    let m = metrics(font);
    let ascent = f64::from(-m.ascent);
    let line = f64::from(m.descent - m.ascent);
    let out = [w, line, ascent, f64::from(m.capHeight), 0.0, 0.0, w, line];
    destroy_font(font, tf);
    out
}
