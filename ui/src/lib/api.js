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
 * @typedef {Object} AppSettings
 * @property {number} version
 * @property {boolean} enabled
 * @property {string} language
 * @property {Exclusions} exclusions
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
