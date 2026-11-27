// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Wiring the buttons and starting up. Loaded last, after everything it uses.

/* ---------- wiring ---------- */
$("open-settings").addEventListener("click", () => openSettings());
$("open-help").addEventListener("click", (e) => { e.stopPropagation(); toggleHelp(); });
for (const b of document.querySelectorAll("[data-help]")) b.addEventListener("click", () => helpAction(b.dataset.help));
$("help-menu").addEventListener("keydown", (e) => {
  const items = [...$("help-menu").querySelectorAll("button")];
  const i = items.indexOf(document.activeElement);
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus();
  }
});
document.addEventListener("click", (e) => { if (!e.target.closest(".menu-wrap")) closeMenus(); });
$("open-palette").addEventListener("click", (e) => { e.preventDefault(); openPalette(); });
$("palette-input").addEventListener("input", () => { state.palette.index = 0; renderPalette(); });
$("palette-input").addEventListener("keydown", paletteKeys);
$("palette").addEventListener("click", (e) => { if (e.target === $("palette")) $("palette").close(); });
for (const b of document.querySelectorAll("[data-close]")) b.addEventListener("click", () => b.closest("dialog").close());
$("whatsnew-tour").addEventListener("click", () => { $("whatsnew-dialog").close(); startTour(); });
$("tour-next").addEventListener("click", () => tourMove(1));
$("tour-back").addEventListener("click", () => tourMove(-1));
$("tour-skip").addEventListener("click", endTour);
$("report-save").addEventListener("click", reportSave);
$("report-copy").addEventListener("click", reportCopy);
$("report-logs").addEventListener("click", reportLogs);
$("report-close").addEventListener("click", () => $("report-dialog").close());
for (const id of ["report-summary", "report-description"]) {
  // Changing the words after saving makes a new file with them.
  $(id).addEventListener("input", () => { if (state.report) state.report.saved = null; });
}
$("update-pill").addEventListener("click", () => openUpdateDialog());
$("update-later").addEventListener("click", () => $("update-dialog").close());
$("update-skip").addEventListener("click", skipThisVersion);
$("check-updates").addEventListener("click", () => checkUpdates(false));
$("roll-back").addEventListener("click", rollBack);
$("copy-command").addEventListener("click", copyCommand);
$("export-settings").addEventListener("click", exportSettings);
$("obs-all").addEventListener("change", (e) => setObsAll(e.target.checked));
for (const r of document.querySelectorAll('input[name="channel"]')) {
  r.addEventListener("change", () => r.checked && setChannel(r.value));
}
$("restore-all").addEventListener("click", restoreAll);
$("uninstall").addEventListener("click", openUninstall);
$("uninstall-cancel").addEventListener("click", () => $("uninstall-dialog").close());
$("uninstall-go").addEventListener("click", uninstallNow);
$("uninstall-close").addEventListener("click", () => { $("uninstall-dialog").close(); closeSettings(); load(); });
$("rail").addEventListener("keydown", railKeys);
$("close-settings").addEventListener("click", closeSettings);
$("run-pc-check").addEventListener("click", runPcCheck);
$("run-self-test").addEventListener("click", runSelfTest);
for (const r of document.querySelectorAll('input[name="theme"]')) {
  r.addEventListener("change", () => r.checked && setAppSetting("theme", r.value));
}
$("big-screen").addEventListener("change", (e) => setAppSetting("big_screen", e.target.checked));
$("protondb").addEventListener("change", (e) => setAppSetting("protondb", e.target.checked));
$("daily-check").addEventListener("change", (e) => setAppSetting("daily_check", e.target.checked));
$("tray").addEventListener("change", (e) => setAppSetting("tray", e.target.checked));
for (const a of document.querySelectorAll(".settings-nav a")) {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    document.querySelector(a.getAttribute("href")).scrollIntoView({ behavior: "smooth", block: "start" });
  });
}
document.addEventListener("keydown", onKey);
window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyDisplay);
$("search").addEventListener("input", (e) => { state.query = e.target.value.trim().toLowerCase(); renderRail(); });

// Errors in the window go into CachyPlay's log, so a bug report includes them.
window.addEventListener("error", (e) => {
  invoke("log_ui_error", { message: `${e.message} (${e.filename}:${e.lineno})` }).catch(() => {});
});
window.addEventListener("unhandledrejection", (e) => {
  invoke("log_ui_error", { message: `unhandled: ${e.reason}` }).catch(() => {});
});

loadAppSettings().then(load).then(showLastCrash).then(afterStart);
startWatching();
// The daily check for a newer CachyPlay (uses a copy of GitHub's answer up to a day old), unless
// turned off in Settings › Updates.
loadAppSettings().then(() => { if (state.settings.daily_check !== false) checkUpdates(true); });
// Quietly check the PC; problems show as a banner.
runPcCheck();
