// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Settings: PC check, self-test, display and privacy, and the game list keys.

/* ---------- PC check and self-test ---------- */
const LEVEL_TEXT = { ok: "OK", warn: "Worth fixing", bad: "Problem", info: "Good to know" };

function checkItem(c) {
  return h("li", { class: `check-item ${c.level}` },
    h("span", { class: "check-level" }, LEVEL_TEXT[c.level]),
    h("div", {},
      h("strong", {}, c.title),
      c.level === "ok" ? h("span", { class: "muted" }, ` ${c.detail}`) : h("p", {}, c.detail),
      c.command ? h("div", { class: "command-row" }, h("code", {}, c.command),
        h("button", { type: "button", class: "secondary", onclick: () => copyText(c.command, "Copied. Paste it into a terminal.") }, "Copy")) : null));
}

function sortChecks(list) {
  const order = { bad: 0, warn: 1, info: 2, ok: 3 };
  return [...list].sort((a, b) => order[a.level] - order[b.level]);
}

async function runPcCheck() {
  try {
    state.pcCheck = await invoke("pc_check");
  } catch (e) {
    state.pcCheck = [];
    toast(String(e), "error");
  }
  $("pc-checks").replaceChildren(...sortChecks(state.pcCheck).map(checkItem));
  if (!$("settings").hidden || !state.overview) return; // the game list draws the banner when it loads
  if (state.selected) renderMain(); else renderEmptyMain();
}

async function runSelfTest() {
  try {
    const list = await invoke("self_test");
    $("self-test").replaceChildren(...sortChecks(list).map(checkItem));
  } catch (e) {
    toast(String(e), "error");
  }
}

/* ---------- display and privacy ---------- */
async function loadAppSettings() {
  try {
    state.settings = await invoke("app_settings");
  } catch {
    state.settings = {};
  }
  applyDisplay();
}

function renderAppSettings() {
  const st = state.settings;
  for (const r of document.querySelectorAll('input[name="theme"]')) r.checked = r.value === (st.theme || "system");
  $("big-screen").checked = !!st.big_screen;
  $("protondb").checked = !!st.protondb;
  $("daily-check").checked = st.daily_check !== false;
  $("tray").checked = !!st.tray;
}

async function setAppSetting(key, value) {
  try {
    await invoke("set_app_setting", { key, value });
    state.settings[key] = value;
  } catch (e) {
    toast(String(e), "error", { label: "OK", run: () => {} });
  }
  renderAppSettings();
  applyDisplay();
  if (key === "protondb" && value && state.selected) showGame(state.selected, { keepScroll: true });
}

/* Theme (follow the desktop, or forced) and KDE's accent color. */
async function applyDisplay() {
  const st = state.settings;
  const theme = st.theme || "system";
  // "Follow my desktop": ask the desktop itself, since the window often reports "light" when it
  // simply doesn't know. If the desktop doesn't say either, use the dark theme.
  let dark = theme === "dark";
  if (theme === "system") {
    let desk = null;
    try {
      desk = await invoke("desktop_theme");
    } catch {
      desk = null;
    }
    dark = desk ? desk === "dark" : true;
  }
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.body.classList.toggle("big-screen", !!st.big_screen);
  // CachyPlay always uses its own accent (aerospace orange), like every app in the family.
  if (st.big_screen) startGamepad();
}

/* Big-screen mode: a controller moves around like the arrow keys. A = press, B = back. */
let gamepadTimer = null;
const padState = {};
function stopGamepad() {
  if (gamepadTimer) clearInterval(gamepadTimer);
  gamepadTimer = null;
}
function startGamepad() {
  if (gamepadTimer || document.hidden || !navigator.getGamepads) return;
  gamepadTimer = setInterval(() => {
    if (!state.settings.big_screen) return;
    for (const pad of navigator.getGamepads()) {
      if (!pad) continue;
      const pressed = (i) => pad.buttons[i] && pad.buttons[i].pressed;
      const axisY = pad.axes[1] || 0;
      const axisX = pad.axes[0] || 0;
      const inputs = {
        up: pressed(12) || axisY < -0.6,
        down: pressed(13) || axisY > 0.6,
        left: pressed(14) || axisX < -0.6,
        right: pressed(15) || axisX > 0.6,
        a: pressed(0),
        b: pressed(1),
        x: pressed(2),
        y: pressed(3),
        lb: pressed(4),
        rb: pressed(5),
      };
      for (const [k, on] of Object.entries(inputs)) {
        const was = padState[k];
        padState[k] = on;
        if (!on || was) continue;
        padPress(k);
      }
    }
  }, 80);
}

function focusables() {
  const root = $("settings").hidden ? $("games-view") : $("settings");
  return [...root.querySelectorAll("button:not([disabled]), a[href], input:not([disabled]), summary")].filter((el) => el.offsetParent !== null);
}

function padPress(k) {
  const el = document.activeElement;
  if (k === "a") { if (el && el !== document.body) el.click(); return; }
  if (k === "b") {
    if (!$("settings").hidden) closeSettings();
    else if (document.querySelector("dialog[open]")) for (const d of document.querySelectorAll("dialog[open]")) d.close();
    else if (state.selected) goHome();
    return;
  }
  // X: Library. Y: report a problem. LB/RB: previous/next game in the list.
  if (k === "x") { closeSettings(); goHome(); return; }
  if (k === "y") { openReport(state.selected ? { appId: state.selected } : {}); return; }
  if (k === "lb" || k === "rb") {
    const ids = [...document.querySelectorAll("#rail .rail-item[data-app]")].map((b) => b.dataset.app);
    if (!ids.length) return;
    const i = ids.indexOf(state.selected);
    const next = ids[i < 0 ? 0 : (i + (k === "rb" ? 1 : ids.length - 1)) % ids.length];
    openGame(next);
    return;
  }
  const list = focusables();
  if (!list.length) return;
  const i = list.indexOf(el);
  // Left/right jump between the game list and the game page; up/down move within.
  if ((k === "left" || k === "right") && $("settings").hidden) {
    const inRail = el && el.closest && el.closest("#rail");
    const target = k === "right" && inRail ? $("main").querySelector("button:not([disabled])")
      : k === "left" && !inRail ? $("rail").querySelector('.rail-item[aria-current="true"]') || $("rail").querySelector(".rail-item")
      : null;
    if (target) { target.focus(); target.scrollIntoView({ block: "nearest" }); }
    return;
  }
  const next = list[Math.max(0, Math.min(list.length - 1, i + (k === "down" || k === "right" ? 1 : -1)))];
  if (next) { next.focus(); next.scrollIntoView({ block: "nearest" }); }
}

async function renderObsSettings() {
  let o;
  try {
    o = await invoke("obs_settings");
  } catch {
    return;
  }
  $("obs-all").checked = o.all;
  const line = $("obs-status");
  if (o.installed) {
    line.className = "status-line good";
    line.replaceChildren("obs-gamecapture is installed.");
  } else {
    line.className = "status-line";
    line.replaceChildren(
      "obs-gamecapture isn't installed. Install it in a terminal: ",
      h("code", {}, o.install_command), " ",
      h("button", { type: "button", class: "link", onclick: () => copyText(o.install_command, "Copied. Paste it into a terminal.") }, "Copy"));
  }
}

async function setObsAll(on) {
  try {
    toast(await invoke("set_obs_all", { on }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  renderObsSettings();
  if (state.selected) showGame(state.selected, { keepScroll: true });
}

async function setObs(appId, on) {
  state.busy = true;
  try {
    toast(await invoke("set_obs_capture", { appId, on }), "success");
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    await load();
  }
}

async function copyText(text, done = "Copied. Paste it into the game's Launch options in Steam.") {
  try {
    await navigator.clipboard.writeText(text);
    toast(done, "success");
  } catch {
    toast("Couldn't copy automatically. Select the line and press Ctrl+C.");
  }
}

async function renderKept() {
  const list = $("kept-list");
  let games = [];
  try {
    games = await invoke("game_settings_all");
  } catch (e) {
    list.replaceChildren(h("li", { class: "empty-row" }, String(e)));
    return;
  }
  $("export-settings").disabled = !games.length;
  if (!games.length) {
    list.replaceChildren(h("li", { class: "empty-row" }, "No game has CachyPlay settings yet. Turn CachyPlay on for a game or apply a suggested fix."));
    return;
  }
  list.replaceChildren(...games.map((g) => h("li", {},
    h("div", { class: "kept-head" },
      h("strong", {}, g.name),
      h("button", { type: "button", class: "secondary small", onclick: () => copyText(g.launch_options) }, "Copy")),
    optionsCode(g.launch_options, "kept-line"),
    h("ul", { class: "kept-settings" }, g.settings.map((st) => h("li", {},
      `${st.label} (${st.source})`,
      h("span", {}, st.why)))))));
}

async function exportSettings() {
  try {
    const path = await invoke("export_settings");
    toast(`Saved ${path.split("/").pop()}`, "success", { label: "Show in folder", run: () => invoke("open_folder", { path }) });
  } catch (e) {
    toast(String(e), "error");
  }
}

async function restoreAll() {
  const ok = await confirmDialog("Restore all games?", "CachyPlay will be turned off for every game and their launch options put back. Your own options stay.", "Restore all games");
  if (!ok) return;
  try {
    const lines = await invoke("restore_all");
    toast(lines.length ? `Restored ${lines.length} game${lines.length === 1 ? "" : "s"}.` : "Nothing to restore.", "success");
  } catch (e) {
    toast(String(e), "error");
  }
  load();
}

/* ---------- keyboard: up/down moves through the game list ---------- */
function railKeys(e) {
  if (!["ArrowUp", "ArrowDown"].includes(e.key)) return;
  const items = [...document.querySelectorAll(".rail-item")];
  const i = items.indexOf(document.activeElement);
  if (i < 0) return;
  const next = items[i + (e.key === "ArrowDown" ? 1 : -1)];
  if (next) {
    e.preventDefault();
    next.focus();
    next.click();
  }
}
