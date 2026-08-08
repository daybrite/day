// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Starter translations for the keys `day new app` scaffolds (docs/localization.md).
//!
//! `day localize add <tag>` copies the default locale verbatim under a translate-me header,
//! which is right for an app's own strings — the CLI cannot know what they mean. It CAN know
//! what the scaffold's own strings mean, because it wrote them. So for the handful of keys the
//! generated sample app shows on its opening screen, a locale added here starts translated
//! rather than starting as English labelled otherwise.
//!
//! Scope is deliberately narrow. A key is listed only if the scaffold ships it AND the phrase is
//! a UI label with one obvious rendering in each language. Anything longer (the panel blurbs)
//! stays an English copy for a human to translate in context.
//!
//! `{app}` is replaced with the project's title before the line is written.

/// The scaffold keys carried here, in the order they are written.
pub const KEYS: &[&str] = &[
    "home_greeting",
    "home_welcome",
    "nav_home",
    "nav_controls",
    "nav_canvas",
    "nav_items",
];

/// `(locale tag, [translation per KEYS])`. A tag absent here simply gets the English copy.
#[rustfmt::skip]
pub const STARTER: &[(&str, [&str; 6])] = &[
    ("ar-SA", ["مرحبًا!", "مرحبًا بك في {app}", "الرئيسية", "عناصر التحكم", "لوحة الرسم", "العناصر"]),
    ("cs-CZ", ["Ahoj!", "Vítejte v {app}", "Domů", "Ovládací prvky", "Plátno", "Položky"]),
    ("de-DE", ["Hallo!", "Willkommen bei {app}", "Start", "Steuerelemente", "Leinwand", "Elemente"]),
    ("es-ES", ["¡Hola!", "Te damos la bienvenida a {app}", "Inicio", "Controles", "Lienzo", "Elementos"]),
    ("fr-FR", ["Bonjour !", "Bienvenue dans {app}", "Accueil", "Contrôles", "Canevas", "Éléments"]),
    ("id-ID", ["Halo!", "Selamat datang di {app}", "Beranda", "Kontrol", "Kanvas", "Item"]),
    ("it-IT", ["Ciao!", "Benvenuto in {app}", "Home", "Controlli", "Tela", "Elementi"]),
    ("ja-JP", ["こんにちは！", "{app} へようこそ", "ホーム", "コントロール", "キャンバス", "アイテム"]),
    ("ko-KR", ["안녕하세요!", "{app}에 오신 것을 환영합니다", "홈", "컨트롤", "캔버스", "항목"]),
    ("ms-MY", ["Helo!", "Selamat datang ke {app}", "Laman Utama", "Kawalan", "Kanvas", "Item"]),
    ("nl-NL", ["Hallo!", "Welkom bij {app}", "Start", "Bedieningselementen", "Canvas", "Items"]),
    ("pl-PL", ["Cześć!", "Witamy w {app}", "Główna", "Kontrolki", "Płótno", "Elementy"]),
    ("pt-BR", ["Olá!", "Boas-vindas ao {app}", "Início", "Controles", "Tela", "Itens"]),
    ("ru-RU", ["Привет!", "Добро пожаловать в {app}", "Главная", "Элементы управления", "Холст", "Элементы"]),
    ("th-TH", ["สวัสดี!", "ยินดีต้อนรับสู่ {app}", "หน้าแรก", "ตัวควบคุม", "ผืนผ้าใบ", "รายการ"]),
    ("tr-TR", ["Merhaba!", "{app} uygulamasına hoş geldiniz", "Ana Sayfa", "Denetimler", "Tuval", "Öğeler"]),
    ("uk-UA", ["Привіт!", "Ласкаво просимо до {app}", "Головна", "Елементи керування", "Полотно", "Елементи"]),
    ("vi-VN", ["Xin chào!", "Chào mừng đến với {app}", "Trang chủ", "Điều khiển", "Canvas", "Mục"]),
    ("zh-Hans-CN", ["你好！", "欢迎使用 {app}", "主页", "控件", "画布", "项目"]),
    ("zh-Hant-TW", ["你好！", "歡迎使用 {app}", "首頁", "控制項", "畫布", "項目"]),
];

/// The starter translations for `tag`, matched on the exact tag then its language subtag, so
/// `de` and `de-AT` both reach the `de-DE` row.
pub fn starter_for(tag: &str) -> Option<&'static [&'static str; 6]> {
    if let Some((_, v)) = STARTER.iter().find(|(t, _)| *t == tag) {
        return Some(v);
    }
    let lang = tag.split('-').next().unwrap_or(tag);
    STARTER
        .iter()
        .find(|(t, _)| t.split('-').next() == Some(lang))
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_covers_every_key() {
        assert_eq!(KEYS.len(), 6);
        for (tag, vals) in STARTER {
            for (k, v) in KEYS.iter().zip(vals) {
                assert!(!v.trim().is_empty(), "{tag}: {k} is empty");
            }
        }
    }

    #[test]
    fn the_welcome_line_keeps_its_placeholder() {
        let i = KEYS.iter().position(|k| *k == "home_welcome").unwrap();
        for (tag, vals) in STARTER {
            assert!(
                vals[i].contains("{app}"),
                "{tag}: home_welcome lost {{app}}"
            );
        }
    }

    /// A language subtag resolves even when the region differs, which is what an app that spells
    /// its locales `de` rather than `de-DE` relies on.
    #[test]
    fn a_bare_language_tag_resolves() {
        assert!(starter_for("ja-JP").is_some());
        assert!(starter_for("ja").is_some());
        assert!(starter_for("de-AT").is_some());
        assert!(starter_for("xx-YY").is_none());
    }
}
