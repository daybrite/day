// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Starter translations for the keys `day new app` scaffolds (docs/localization.md).
//!
//! `day localize add <tag>` copies the default locale verbatim under a translate-me header,
//! which is right for an app's own strings — the CLI cannot know what they mean. It CAN know
//! what the scaffold's own strings mean, because it wrote them. So for the handful of keys the
//! generated sample app shows on its opening screen, a locale added here starts translated
//! rather than starting as English labeled otherwise.
//!
//! Scope is deliberately narrow. A key is listed only if the scaffold ships it AND the phrase is
//! a UI label with one obvious rendering in each language. Anything longer (the panel blurbs)
//! stays an English copy for a human to translate in context.
//!
//! `{app}` is replaced with the project's title before the line is written.

/// The scaffold keys carried here, in the order they are written.
pub const KEYS: &[&str] = &[
    "nav_welcome",
    "nav_navigate",
    "nav_settings",
    "welcome_title",
];

/// `(locale tag, [translation per KEYS])`. A tag absent here simply gets the English copy.
#[rustfmt::skip]
pub const STARTER: &[(&str, [&str; 4])] = &[
    ("ar-SA", ["مرحبًا", "تصفح", "الإعدادات", "مرحبًا بك في Day"]),
    ("cs-CZ", ["Vítejte", "Procházet", "Nastavení", "Vítejte v Day"]),
    ("de-DE", ["Willkommen", "Navigieren", "Einstellungen", "Willkommen bei Day"]),
    ("es-ES", ["Bienvenida", "Navegar", "Ajustes", "Te damos la bienvenida a Day"]),
    ("fr-FR", ["Bienvenue", "Naviguer", "Réglages", "Bienvenue dans Day"]),
    ("id-ID", ["Selamat Datang", "Navigasi", "Pengaturan", "Selamat datang di Day"]),
    ("it-IT", ["Benvenuto", "Naviga", "Impostazioni", "Benvenuto in Day"]),
    ("ja-JP", ["ようこそ", "ナビゲート", "設定", "Day へようこそ"]),
    ("ko-KR", ["환영합니다", "탐색", "설정", "Day에 오신 것을 환영합니다"]),
    ("ms-MY", ["Selamat Datang", "Navigasi", "Tetapan", "Selamat datang ke Day"]),
    ("nl-NL", ["Welkom", "Navigeren", "Instellingen", "Welkom bij Day"]),
    ("pl-PL", ["Powitanie", "Nawigacja", "Ustawienia", "Witamy w Day"]),
    ("pt-BR", ["Bem-vindo", "Navegar", "Configurações", "Boas-vindas ao Day"]),
    ("ru-RU", ["Добро пожаловать", "Навигация", "Настройки", "Добро пожаловать в Day"]),
    ("th-TH", ["ยินดีต้อนรับ", "นำทาง", "การตั้งค่า", "ยินดีต้อนรับสู่ Day"]),
    ("tr-TR", ["Hoş Geldiniz", "Gezin", "Ayarlar", "Day uygulamasına hoş geldiniz"]),
    ("uk-UA", ["Ласкаво просимо", "Навігація", "Налаштування", "Ласкаво просимо до Day"]),
    ("vi-VN", ["Chào mừng", "Điều hướng", "Cài đặt", "Chào mừng đến với Day"]),
    ("zh-Hans-CN", ["欢迎", "导航", "设置", "欢迎使用 Day"]),
    ("zh-Hant-TW", ["歡迎", "導覽", "設定", "歡迎使用 Day"]),
];

/// The starter translations for `tag`, matched on the exact tag then its language subtag, so
/// `de` and `de-AT` both reach the `de-DE` row.
pub fn starter_for(tag: &str) -> Option<&'static [&'static str; 4]> {
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
        assert_eq!(KEYS.len(), 4);
        for (tag, vals) in STARTER {
            for (k, v) in KEYS.iter().zip(vals) {
                assert!(!v.trim().is_empty(), "{tag}: {k} is empty");
            }
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
