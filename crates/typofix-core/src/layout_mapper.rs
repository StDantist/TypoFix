//! Розкладка клавіатури як чисті дані + мапінг у обидва боки.
//!
//! Тут немає жодного IO — лише типи й функції. Дані розкладок завантажує
//! `typofix-data` з TOML і будує [`Layout`] через [`Layout::new`]. У рантаймі
//! продакшн-мапінг братиметься з ОС; ця статична мапа — еталон, fallback і
//! живлення тестів/детектора (див. `data/CLAUDE.md`, `docs/ARCHITECTURE.md` §6).
//!
//! ## Конвенція scancode
//! Скрізь використовуємо **Windows scancode set 1** (make-коди), напр.
//! `Q = 0x10`, `A = 0x1E`, `G = 0x22`, пробіл `= 0x39`. Це **фізична** позиція
//! клавіші, незалежна від активної розкладки — саме тому ту саму послідовність
//! натискань можна «прочитати» в іншій розкладці (ядро детектора). macOS-бекенд
//! зобов'язаний транслювати свої keycode у цю ж конвенцію перед подачею в ядро.

use std::collections::{BTreeMap, HashMap};

use typofix_platform::{KeyEvent, LayoutId, Modifiers};

/// Символи, які дає одна фізична клавіша на трьох рівнях.
///
/// `normal` — без модифікаторів; `shift` — з Shift; `altgr` — з AltGr (третій
/// рівень, напр. для розкладок із додатковими символами). `shift`/`altgr`
/// опційні: якщо клавіша не має окремого символу на рівні, він відсутній.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCap {
    /// Символ без модифікаторів.
    pub normal: char,
    /// Символ із Shift (для літер — велика; може бути власний символ).
    pub shift: Option<char>,
    /// Символ із AltGr (третій рівень розкладки).
    pub altgr: Option<char>,
}

impl KeyCap {
    /// Літерна клавіша: `normal`/`shift` — мала/велика, без AltGr.
    pub fn letter(lower: char, upper: char) -> Self {
        Self {
            normal: lower,
            shift: Some(upper),
            altgr: None,
        }
    }
}

/// Одне фізичне натискання: scancode + модифікатори в момент натиску.
///
/// Це layout-незалежна одиниця: буфер ядра накопичує саме `KeyStroke`, а потім
/// «читає» їх у різних розкладках через [`Layout::interpret`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyStroke {
    /// Scancode set 1 (див. конвенцію в доці модуля).
    pub scancode: u32,
    /// Активні модифікатори (нас цікавлять SHIFT / ALTGR / CAPS).
    pub modifiers: Modifiers,
}

impl KeyStroke {
    /// Створити натискання.
    pub fn new(scancode: u32, modifiers: Modifiers) -> Self {
        Self {
            scancode,
            modifiers,
        }
    }
}

impl From<&KeyEvent> for KeyStroke {
    fn from(ev: &KeyEvent) -> Self {
        Self {
            scancode: ev.scancode,
            modifiers: ev.modifiers,
        }
    }
}

/// Розкладка: мапа `scancode → символи` плюс готовий зворотний індекс
/// `символ → натискання`.
///
/// Будується раз через [`Layout::new`]; обидва напрями мапінгу — O(1)/O(log n).
#[derive(Debug, Clone)]
pub struct Layout {
    id: LayoutId,
    /// `scancode → KeyCap`. BTreeMap — детермінований порядок ітерації.
    keys: BTreeMap<u32, KeyCap>,
    /// `символ → як його набрати`. Перша (найпростіша) інтерпретація виграє.
    reverse: HashMap<char, KeyStroke>,
}

impl Layout {
    /// Зібрати розкладку з пар `(scancode, KeyCap)`.
    ///
    /// Зворотний індекс будується автоматично: для кожної клавіші реєструються
    /// її `normal`/`shift`/`altgr` символи. За колізії (той самий символ на
    /// кількох клавішах/рівнях) перемагає **перший** доданий — а в межах
    /// клавіші пріоритет `normal > shift > altgr` (найпростіший спосіб набору).
    pub fn new(id: LayoutId, caps: impl IntoIterator<Item = (u32, KeyCap)>) -> Self {
        let keys: BTreeMap<u32, KeyCap> = caps.into_iter().collect();
        let mut reverse: HashMap<char, KeyStroke> = HashMap::new();
        // Ітеруємо у детермінованому порядку scancode, щоб індекс був стабільним.
        for (&scancode, cap) in &keys {
            reverse
                .entry(cap.normal)
                .or_insert_with(|| KeyStroke::new(scancode, Modifiers::empty()));
            if let Some(s) = cap.shift {
                reverse
                    .entry(s)
                    .or_insert_with(|| KeyStroke::new(scancode, Modifiers::SHIFT));
            }
            if let Some(a) = cap.altgr {
                reverse
                    .entry(a)
                    .or_insert_with(|| KeyStroke::new(scancode, Modifiers::ALTGR));
            }
        }
        Self { id, keys, reverse }
    }

    /// Ідентифікатор розкладки.
    pub fn id(&self) -> &LayoutId {
        &self.id
    }

    /// Кількість визначених клавіш.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Чи немає жодної клавіші.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// `KeyCap` для scancode, якщо клавіша визначена.
    pub fn keycap(&self, scancode: u32) -> Option<&KeyCap> {
        self.keys.get(&scancode)
    }

    /// Символ, який дасть це натискання в цій розкладці.
    ///
    /// Враховує SHIFT, ALTGR і CAPS. Модель CAPS навмисно проста й коректна для
    /// алфавітних розкладок: Caps Lock інвертує регістр **лише літер**, тож для
    /// літери ефективний регістр = `SHIFT XOR CAPS`; на нелітерні символи CAPS
    /// не впливає. AltGr (якщо для клавіші є `altgr`) має пріоритет.
    pub fn char_at(&self, scancode: u32, modifiers: Modifiers) -> Option<char> {
        let cap = self.keys.get(&scancode)?;
        if modifiers.contains(Modifiers::ALTGR) {
            if let Some(a) = cap.altgr {
                return Some(a);
            }
        }
        let shift = modifiers.contains(Modifiers::SHIFT);
        let caps = modifiers.contains(Modifiers::CAPS);
        let want_upper = if cap.normal.is_alphabetic() {
            shift ^ caps
        } else {
            shift
        };
        if want_upper {
            // Є явний shift-символ → беремо його; інакше для літери піднімаємо
            // регістр самотужки, для решти лишаємо normal.
            match cap.shift {
                Some(s) => Some(s),
                None if cap.normal.is_alphabetic() => {
                    cap.normal.to_uppercase().next().or(Some(cap.normal))
                }
                None => Some(cap.normal),
            }
        } else {
            Some(cap.normal)
        }
    }

    /// Як набрати символ у цій розкладці (scancode + модифікатор), якщо можливо.
    pub fn stroke_for(&self, ch: char) -> Option<KeyStroke> {
        self.reverse.get(&ch).copied()
    }

    /// Прочитати послідовність фізичних натискань як текст у цій розкладці.
    ///
    /// Це і є «альтернативна інтерпретація»: ті самі `KeyStroke` (зняті, коли
    /// користувач набирав в іншій розкладці) можна подати сюди, щоб дізнатися,
    /// що вийшло б у цій. Невідомі scancode пропускаються.
    pub fn interpret(&self, strokes: &[KeyStroke]) -> String {
        strokes
            .iter()
            .filter_map(|s| self.char_at(s.scancode, s.modifiers))
            .collect()
    }

    /// Перекодувати текст із цієї розкладки у `target`: символ → натискання тут
    /// → символ у `target`. Символи, яких немає в цій розкладці, переносяться
    /// як є (напр. пробіл/пунктуація поза мапою). Зручно для тестів і дебагу.
    pub fn transliterate_to(&self, text: &str, target: &Layout) -> String {
        text.chars()
            .map(|ch| {
                self.stroke_for(ch)
                    .and_then(|s| target.char_at(s.scancode, s.modifiers))
                    .unwrap_or(ch)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Мінімальні розкладки для тестів (лише потрібні клавіші, scancode set 1).
    // Загальні фізичні позиції: A=0x1E S=0x1F D=0x20 G=0x22 H=0x23 B=0x30 N=0x31.

    fn en_layout() -> Layout {
        let keys = [
            (0x1E, KeyCap::letter('a', 'A')),
            (0x1F, KeyCap::letter('s', 'S')),
            (0x20, KeyCap::letter('d', 'D')),
            (0x22, KeyCap::letter('g', 'G')),
            (0x23, KeyCap::letter('h', 'H')),
            (0x30, KeyCap::letter('b', 'B')),
            (0x31, KeyCap::letter('n', 'N')),
            (
                0x39,
                KeyCap {
                    normal: ' ',
                    shift: None,
                    altgr: None,
                },
            ),
        ];
        Layout::new(LayoutId::new("en"), keys)
    }

    fn uk_layout() -> Layout {
        let keys = [
            (0x1E, KeyCap::letter('ф', 'Ф')),
            (0x1F, KeyCap::letter('і', 'І')),
            (0x20, KeyCap::letter('в', 'В')),
            (0x22, KeyCap::letter('п', 'П')),
            (0x23, KeyCap::letter('р', 'Р')),
            (0x30, KeyCap::letter('и', 'И')),
            (0x31, KeyCap::letter('т', 'Т')),
            (0x2B, KeyCap::letter('ґ', 'Ґ')), // \ — ґ у розширеній розкладці
            (
                0x29, // grave — апостроф ’ (U+2019)
                KeyCap {
                    normal: '’',
                    shift: Some('₴'),
                    altgr: None,
                },
            ),
            (0x1B, KeyCap::letter('ї', 'Ї')), // ]
            (0x28, KeyCap::letter('є', 'Є')), // '
        ];
        Layout::new(LayoutId::new("uk"), keys)
    }

    fn strokes(scancodes: &[u32]) -> Vec<KeyStroke> {
        scancodes
            .iter()
            .map(|&sc| KeyStroke::new(sc, Modifiers::empty()))
            .collect()
    }

    #[test]
    fn scancode_to_char_roundtrip() {
        let en = en_layout();
        for &sc in &[0x1E, 0x1F, 0x20, 0x22, 0x23, 0x30, 0x31] {
            let ch = en.char_at(sc, Modifiers::empty()).unwrap();
            let stroke = en.stroke_for(ch).unwrap();
            assert_eq!(stroke.scancode, sc, "round-trip для {ch}");
            assert_eq!(en.char_at(stroke.scancode, stroke.modifiers), Some(ch));
        }
    }

    #[test]
    fn shift_gives_uppercase() {
        let en = en_layout();
        assert_eq!(en.char_at(0x1E, Modifiers::empty()), Some('a'));
        assert_eq!(en.char_at(0x1E, Modifiers::SHIFT), Some('A'));
        // Зворотний індекс знає, що 'A' = Shift+A.
        let s = en.stroke_for('A').unwrap();
        assert_eq!(s, KeyStroke::new(0x1E, Modifiers::SHIFT));
    }

    #[test]
    fn caps_inverts_letters_only() {
        let en = en_layout();
        // CAPS на літері = велика; CAPS+SHIFT = мала.
        assert_eq!(en.char_at(0x1E, Modifiers::CAPS), Some('A'));
        assert_eq!(
            en.char_at(0x1E, Modifiers::CAPS | Modifiers::SHIFT),
            Some('a')
        );
        // CAPS не впливає на пробіл (нелітерний).
        assert_eq!(en.char_at(0x39, Modifiers::CAPS), Some(' '));
    }

    #[test]
    fn ukrainian_apostrophe_and_g_present() {
        let uk = uk_layout();
        // Апостроф ’ — це U+2019, а не ASCII '.
        assert_eq!(uk.char_at(0x29, Modifiers::empty()), Some('\u{2019}'));
        assert!(uk.stroke_for('’').is_some());
        // ґ / Ґ
        assert_eq!(uk.char_at(0x2B, Modifiers::empty()), Some('ґ'));
        assert_eq!(uk.char_at(0x2B, Modifiers::SHIFT), Some('Ґ'));
        // і / ї / є
        assert_eq!(uk.char_at(0x1F, Modifiers::empty()), Some('і'));
        assert_eq!(uk.char_at(0x1B, Modifiers::empty()), Some('ї'));
        assert_eq!(uk.char_at(0x28, Modifiers::empty()), Some('є'));
    }

    #[test]
    fn alternative_interpretation_ghbdsn_is_privit() {
        // Фізична послідовність g h b d s n (set 1):
        let seq = strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]);
        let en = en_layout();
        let uk = uk_layout();
        assert_eq!(en.interpret(&seq), "ghbdsn");
        assert_eq!(uk.interpret(&seq), "привіт");
    }

    #[test]
    fn transliterate_between_layouts() {
        let en = en_layout();
        let uk = uk_layout();
        // Те, що користувач хотів набрати "привіт", але був у en → "ghbdsn".
        assert_eq!(uk.transliterate_to("привіт", &en), "ghbdsn");
    }
}
