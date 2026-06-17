// WebdriverIO + tauri-driver (WebView2) для UI-e2e TypoFix.
// Запуск: `npm run test:e2e` з теки `ui/e2e`.
//
// Як це працює:
//   wdio → tauri-driver (intermediary WebDriver) → msedgedriver (native) →
//   запущений `typofix-app.exe` з env `TYPOFIX_E2E=1` (вікно видиме, движок/хуки OFF).
//
// Передумови (див. README.md):
//   • зібраний `src-tauri/target/release/typofix-app.exe` (`tauri build --no-bundle`);
//   • `tauri-driver` у `~/.cargo/bin` (`cargo install tauri-driver`);
//   • `msedgedriver.exe` у `./drivers/`, версія = версії WebView2 Runtime.

const os = require("os");
const path = require("path");
const { spawn } = require("child_process");

const PROJECT_ROOT = path.resolve(__dirname, "..", "..");
const APP_BINARY = path.resolve(
  PROJECT_ROOT,
  "src-tauri",
  "target",
  "release",
  "typofix-app.exe",
);
const TAURI_DRIVER = path.resolve(
  os.homedir(),
  ".cargo",
  "bin",
  "tauri-driver.exe",
);
const MSEDGEDRIVER = path.resolve(__dirname, "drivers", "msedgedriver.exe");

// Запущений у beforeSession; вбитий у afterSession.
let tauriDriver;

exports.config = {
  runner: "local",
  specs: ["./specs/**/*.e2e.js"],
  maxInstances: 1,

  capabilities: [
    {
      // tauri-driver розпізнає застосунок за `tauri:options.application`.
      "tauri:options": {
        application: APP_BINARY,
        // Прокидаємо тест-режим у середовище самого застосунку.
        env: { TYPOFIX_E2E: "1" },
      },
    },
  ],

  // tauri-driver слухає на 4444 за замовчуванням.
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",

  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    timeout: 60000,
  },

  // Резерв: деякі версії tauri-driver не прокидають `tauri:options.env`,
  // тож дублюємо прапорець у власне середовище процесу-драйвера (успадковується
  // запущеним застосунком).
  beforeSession: () => {
    tauriDriver = spawn(
      TAURI_DRIVER,
      ["--native-driver", MSEDGEDRIVER],
      {
        stdio: [null, process.stdout, process.stderr],
        env: { ...process.env, TYPOFIX_E2E: "1" },
      },
    );
  },

  afterSession: () => {
    if (tauriDriver) tauriDriver.kill();
  },
};
