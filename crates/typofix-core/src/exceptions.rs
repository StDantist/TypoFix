//! Виключення застосунків/папок: де TypoFix **взагалі не чіпає** ввід.
//!
//! Чисто й детерміновано — це лише дані + матчинг. Правила передаються в
//! [`Context`](crate::Context) позиченими; core нічого не вантажить.
//!
//! Три види виключень (за зростанням широти):
//! - **process_name** — напр. `game.exe`;
//! - **повний exe-шлях** — конкретний бінар;
//! - **префікс шляху = ціла папка** — будь-який exe з-під теки (рекурсивно),
//!   напр. `C:\Games\…`. Це durable-рішення: «виключити папку» = exe-prefix.
//!
//! ## Готча: нормалізація шляхів
//! Матчинг **орієнтований на Windows**: шляхи зводяться до нижнього регістру
//! (Windows-FS регістронезалежна) і `/`→`\`. На macOS файлова система зазвичай
//! регістрочутлива — там нормалізацію регістру доведеться переглянути (TODO для
//! платформного шару). Префікс папки матчиться **по межі сепаратора**, тож
//! `C:\Games` НЕ зачіпає `C:\GamesX\a.exe`.

use typofix_platform::WindowInfo;

/// Звести шлях до канонічної форми для матчингу (lowercase + `/`→`\`).
fn normalize_path(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// Чи `exe` лежить під текою `folder` (обидва вже нормалізовані; `folder` без
/// хвостового сепаратора). Збіг лише по межі сепаратора.
fn is_under_folder(exe: &str, folder: &str) -> bool {
    match exe.strip_prefix(folder) {
        Some(rest) => rest.is_empty() || rest.starts_with('\\'),
        None => false,
    }
}

/// Набір правил виключення вікон.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionRules {
    /// Імена процесів (нижній регістр), напр. `"game.exe"`.
    process_names: Vec<String>,
    /// Повні exe-шляхи (нормалізовані).
    exe_paths: Vec<String>,
    /// Префікси-теки (нормалізовані, без хвостового `\`).
    folders: Vec<String>,
}

impl ExclusionRules {
    /// Порожній набір (const → придатний для `static`).
    pub const fn new() -> Self {
        Self {
            process_names: Vec::new(),
            exe_paths: Vec::new(),
            folders: Vec::new(),
        }
    }

    /// Виключити за іменем процесу (регістронезалежно).
    pub fn exclude_process(&mut self, name: &str) -> &mut Self {
        self.process_names.push(name.to_lowercase());
        self
    }

    /// Виключити конкретний exe за повним шляхом.
    pub fn exclude_exe(&mut self, path: &str) -> &mut Self {
        self.exe_paths.push(normalize_path(path));
        self
    }

    /// Виключити цілу теку (всі exe з-під неї, рекурсивно).
    pub fn exclude_folder(&mut self, path: &str) -> &mut Self {
        let norm = normalize_path(path);
        self.folders.push(norm.trim_end_matches('\\').to_string());
        self
    }

    /// Чи активне вікно підпадає під будь-яке виключення.
    pub fn excludes(&self, window: &WindowInfo) -> bool {
        let pname = window.process_name.to_lowercase();
        if !pname.is_empty() && self.process_names.iter().any(|n| n == &pname) {
            return true;
        }
        if window.exe_path.is_empty() {
            return false;
        }
        let exe = normalize_path(&window.exe_path);
        self.exe_paths.iter().any(|p| p == &exe)
            || self.folders.iter().any(|f| is_under_folder(&exe, f))
    }

    /// Чи набір порожній (нічого не виключає).
    pub fn is_empty(&self) -> bool {
        self.process_names.is_empty() && self.exe_paths.is_empty() && self.folders.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(process: &str, exe: &str) -> WindowInfo {
        WindowInfo {
            process_name: process.to_string(),
            exe_path: exe.to_string(),
            is_fullscreen: false,
        }
    }

    #[test]
    fn empty_excludes_nothing() {
        let rules = ExclusionRules::new();
        assert!(!rules.excludes(&window("game.exe", r"C:\Games\game.exe")));
    }

    #[test]
    fn process_name_match_is_case_insensitive() {
        let mut rules = ExclusionRules::new();
        rules.exclude_process("Game.exe");
        assert!(rules.excludes(&window("game.exe", "")));
        assert!(rules.excludes(&window("GAME.EXE", r"D:\x\game.exe")));
        assert!(!rules.excludes(&window("editor.exe", "")));
    }

    #[test]
    fn full_exe_path_match_normalizes_separators_and_case() {
        let mut rules = ExclusionRules::new();
        rules.exclude_exe(r"C:\Apps\Secret\tool.exe");
        // Інший регістр і прямі слеші — той самий шлях.
        assert!(rules.excludes(&window("tool.exe", "c:/apps/secret/tool.exe")));
        assert!(!rules.excludes(&window("tool.exe", r"C:\Apps\Other\tool.exe")));
    }

    #[test]
    fn folder_prefix_excludes_nested_exe() {
        let mut rules = ExclusionRules::new();
        rules.exclude_folder(r"C:\Games");
        assert!(rules.excludes(&window("a.exe", r"C:\Games\a.exe")));
        assert!(rules.excludes(&window("b.exe", r"C:\Games\sub\deep\b.exe")));
        // Регістр/слеші теж нормалізуються.
        assert!(rules.excludes(&window("c.exe", "c:/games/c.exe")));
    }

    #[test]
    fn folder_prefix_respects_separator_boundary() {
        let mut rules = ExclusionRules::new();
        rules.exclude_folder(r"C:\Games");
        // Сусідня тека з тим самим префіксом-рядком — НЕ виключена.
        assert!(!rules.excludes(&window("x.exe", r"C:\GamesX\x.exe")));
        assert!(!rules.excludes(&window("x.exe", r"C:\GamesBackup\x.exe")));
    }

    #[test]
    fn folder_with_trailing_separator_is_handled() {
        let mut rules = ExclusionRules::new();
        rules.exclude_folder(r"C:\Games\");
        assert!(rules.excludes(&window("a.exe", r"C:\Games\a.exe")));
        assert!(!rules.excludes(&window("x.exe", r"C:\GamesX\x.exe")));
    }
}
