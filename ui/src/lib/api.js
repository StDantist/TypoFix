// Тонка обгортка над Tauri-командами конфігу та діалогом вибору файлів.
// Винесено сюди, щоб компоненти не знали деталей IPC.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

/**
 * @typedef {Object} Exclusions
 * @property {string[]} process_names
 * @property {string[]} exe_paths
 * @property {string[]} folders
 *
 * @typedef {Object} Detection
 * @property {number} min_word_len
 * @property {number} confidence_threshold
 *
 * @typedef {Object} HotkeyBinding
 * @property {string} accelerator  рядок-акселератор у форматі Tauri, напр. "Ctrl+Alt+P"
 * @property {boolean} enabled
 *
 * @typedef {Object} Hotkeys
 * @property {HotkeyBinding} pause_resume
 * @property {HotkeyBinding} revert_last
 * @property {HotkeyBinding} manual_switch
 * @property {HotkeyBinding} case_upper
 * @property {HotkeyBinding} case_lower
 * @property {HotkeyBinding} case_sentence
 *
 * @typedef {Object} Words
 * @property {string[]} always_switch
 * @property {string[]} never_switch
 *
 * @typedef {Object} Behavior
 * @property {boolean} fix_case
 * @property {boolean} forex
 * @property {boolean} recognize_extensions
 * @property {boolean} phonotactics
 * @property {boolean} fix_capslock
 *
 * @typedef {Object} AppSettings
 * @property {number} version
 * @property {boolean} enabled
 * @property {string} language
 * @property {Exclusions} exclusions
 * @property {Words} words
 * @property {Hotkeys} hotkeys
 * @property {Behavior} behavior
 * @property {Detection} detection
 */

/** Прочитати конфіг із диска (джерело істини). @returns {Promise<AppSettings>} */
export function loadSettings() {
  return invoke("load_settings");
}

/**
 * Зберегти конфіг. Бекенд валідує й повертає очищену версію.
 * @param {AppSettings} settings
 * @returns {Promise<AppSettings>}
 */
export function saveSettings(settings) {
  return invoke("save_settings", { settings });
}

/**
 * @typedef {Object} ProcessEntry
 * @property {string} name      exe-ім'я, напр. "chrome.exe"
 * @property {string|null} exe_path повний шлях, якщо доступний
 * @property {string|null} icon  base64 PNG data-URL іконки exe, якщо вдалось витягти
 * @property {boolean} has_window чи має застосунок видиме верхньорівневе вікно
 */

/**
 * Перелік зараз запущених процесів (дедуп за exe-іменем, сортовано).
 * @returns {Promise<ProcessEntry[]>}
 */
export function listRunningProcesses() {
  return invoke("list_running_processes");
}

/** Діалог вибору теки. @returns {Promise<string|null>} */
export async function pickFolder() {
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

/** Діалог вибору .exe. @returns {Promise<string|null>} */
export async function pickExe() {
  const res = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Виконувані файли", extensions: ["exe"] }],
  });
  return typeof res === "string" ? res : null;
}
