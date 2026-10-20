// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Starter translations for the keys `day new app` scaffolds (docs/localization.md).
//!
//! `day localize add <tag>` copies the default locale verbatim under a translate-me header,
//! which is right for an app's strings: the CLI cannot know what they mean. It can know
//! what the scaffold's strings mean, because it wrote them. So for the handful of keys the
//! sample app uses for navigation and empty states, a locale added here starts translated
//! rather than starting as English labeled otherwise.
//!
//! Scope is narrow: starter labels and short empty-state messages. Longer prose stays
//! an English copy for a human to translate in context.
//!
//! `{app}` is replaced with the project's title before the line is written.

/// The scaffold keys carried here, in the order they are written.
pub const KEYS: &[&str] = &[
    "nav_welcome",
    "welcome_title",
    "nav_items",
    "items_empty",
    "item_untitled",
];

/// One translation for each key.
pub type Translations = [&'static str; KEYS.len()];

/// `(locale tag, [translation per KEYS])`. A tag absent here gets the English copy.
#[rustfmt::skip]
pub const STARTER: &[(&str, Translations)] = &[
    ("ar-SA", ["مرحبًا", "مرحبًا بك في Day", "العناصر", "لا توجد عناصر بعد. أضف عنصرك الأول.", "بلا عنوان"]),
    ("cs-CZ", ["Vítejte", "Vítejte v Day", "Položky", "Zatím tu nejsou žádné položky. Přidejte první.", "Bez názvu"]),
    ("de-DE", ["Willkommen", "Willkommen bei Day", "Einträge", "Noch keine Einträge. Füge deinen ersten Eintrag hinzu.", "Ohne Titel"]),
    ("es-ES", ["Bienvenida", "Te damos la bienvenida a Day", "Elementos", "Aún no hay elementos. Añade el primero.", "Sin título"]),
    ("fr-FR", ["Bienvenue", "Bienvenue dans Day", "Éléments", "Aucun élément pour le moment. Ajoutez votre premier élément.", "Sans titre"]),
    ("id-ID", ["Selamat Datang", "Selamat datang di Day", "Item", "Belum ada item. Tambahkan item pertama Anda.", "Tanpa judul"]),
    ("it-IT", ["Benvenuto", "Benvenuto in Day", "Elementi", "Non ci sono ancora elementi. Aggiungi il primo.", "Senza titolo"]),
    ("ja-JP", ["ようこそ", "Day へようこそ", "項目", "項目はまだありません。最初の項目を追加しましょう。", "無題"]),
    ("ko-KR", ["환영합니다", "Day에 오신 것을 환영합니다", "항목", "아직 항목이 없습니다. 첫 항목을 추가하세요.", "제목 없음"]),
    ("ms-MY", ["Selamat Datang", "Selamat datang ke Day", "Item", "Belum ada item. Tambahkan item pertama anda.", "Tanpa tajuk"]),
    ("nl-NL", ["Welkom", "Welkom bij Day", "Items", "Nog geen items. Voeg je eerste item toe.", "Naamloos"]),
    ("pl-PL", ["Powitanie", "Witamy w Day", "Elementy", "Nie ma jeszcze żadnych elementów. Dodaj pierwszy.", "Bez tytułu"]),
    ("pt-BR", ["Bem-vindo", "Boas-vindas ao Day", "Itens", "Ainda não há itens. Adicione seu primeiro item.", "Sem título"]),
    ("ru-RU", ["Добро пожаловать", "Добро пожаловать в Day", "Элементы", "Пока нет элементов. Добавьте первый.", "Без названия"]),
    ("th-TH", ["ยินดีต้อนรับ", "ยินดีต้อนรับสู่ Day", "รายการ", "ยังไม่มีรายการ เพิ่มรายการแรกของคุณ", "ไม่มีชื่อ"]),
    ("tr-TR", ["Hoş Geldiniz", "Day uygulamasına hoş geldiniz", "Öğeler", "Henüz öğe yok. İlk öğenizi ekleyin.", "Başlıksız"]),
    ("uk-UA", ["Ласкаво просимо", "Ласкаво просимо до Day", "Елементи", "Ще немає елементів. Додайте перший.", "Без назви"]),
    ("vi-VN", ["Chào mừng", "Chào mừng đến với Day", "Mục", "Chưa có mục nào. Hãy thêm mục đầu tiên.", "Chưa có tiêu đề"]),
    ("zh-Hans-CN", ["欢迎", "欢迎使用 Day", "项目", "暂无项目。添加你的第一个项目吧。", "无标题"]),
    ("zh-Hant-TW", ["歡迎", "歡迎使用 Day", "項目", "尚無項目。新增你的第一個項目吧。", "無標題"]),
];

/// The starter translations for `tag`, matched on the exact tag then its language subtag, so
/// `de` and `de-AT` both reach the `de-DE` row.
pub fn starter_for(tag: &str) -> Option<&'static Translations> {
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
        assert_eq!(STARTER.len(), 20);
        assert_eq!(
            KEYS.iter().collect::<std::collections::HashSet<_>>().len(),
            KEYS.len()
        );
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
