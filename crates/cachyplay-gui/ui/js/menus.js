// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Icons, the Help menu, shortcuts, Ctrl+K, the tour, What's new, About and uninstall.

/* ---------- icons (Lucide, from the sprite in index.html) ---------- */
function icon(name, size = 16) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("class", "icon");
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(ns, "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

/* ---------- Help menu ---------- */
function closeMenus() {
  $("help-menu").hidden = true;
  $("open-help").setAttribute("aria-expanded", "false");
}

function toggleHelp() {
  const m = $("help-menu");
  const open = m.hidden;
  closeMenus();
  if (open) {
    m.hidden = false;
    $("open-help").setAttribute("aria-expanded", "true");
    m.querySelector("button").focus();
  }
}

function helpAction(what) {
  closeMenus();
  if (what === "report") openReport({ appId: state.selected });
  else if (what === "whatsnew") openWhatsNew();
  else if (what === "shortcuts") openShortcuts();
  else if (what === "tour") startTour();
  else if (what === "about") openAbout();
}

/* ---------- keyboard shortcuts ---------- */
const SHORTCUTS = [
  ["Ctrl+K", "Search games, fixes, settings and actions"],
  ["Ctrl+,", "Settings"],
  ["?", "This list of shortcuts"],
  ["F1", "Help menu"],
  ["/", "Search games"],
  ["↑ ↓", "Move through the game list"],
  ["Esc", "Close a dialog, or go back from Settings"],
  ["Ctrl+Q", "Quit CachyPlay"],
];

function openShortcuts() {
  $("shortcut-list").replaceChildren(...SHORTCUTS.flatMap(([k, what]) => [
    h("dt", {}, ...k.split("+").map((p, i) => [i ? "+" : null, h("kbd", {}, p)]).flat()),
    h("dd", {}, what)]));
  $("shortcuts-dialog").showModal();
}

function typingIn(el) {
  return el && (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || el.isContentEditable);
}

function onKey(e) {
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl && e.key.toLowerCase() === "k") { e.preventDefault(); openPalette(); return; }
  if (ctrl && e.key === ",") { e.preventDefault(); openSettings(); return; }
  if (ctrl && e.key.toLowerCase() === "q") { e.preventDefault(); invoke("quit_app"); return; }
  if (e.key === "F1") { e.preventDefault(); toggleHelp(); return; }
  if (e.key === "Escape") {
    if (!$("help-menu").hidden) { closeMenus(); $("open-help").focus(); return; }
    if (!$("tour").hidden) { endTour(); return; }
    if (!$("settings").hidden && !document.querySelector("dialog[open]")) closeSettings();
    return;
  }
  if (typingIn(document.activeElement) || ctrl || e.altKey || document.querySelector("dialog[open]")) return;
  if (e.key === "?") { e.preventDefault(); openShortcuts(); return; }
  if (e.key === "/") { e.preventDefault(); closeSettings(); $("search").focus(); }
}

/* ---------- Ctrl+K: search everything ---------- */
const SETTING_SECTIONS = [
  ["set-general", "General", "tray icon notifications"],
  ["set-appearance", "Appearance", "theme dark light big screen controller tv"],
  ["set-updates", "Updates", "stable beta channel check version go back"],
  ["set-privacy", "Privacy", "protondb bug reports saved"],
  ["set-pc", "PC check", "problems driver"],
  ["set-obs", "OBS game capture", "record stream"],
  ["set-kept", "Your game settings", "launch options export"],
  ["set-advanced", "Advanced", "self-test restore all"],
  ["set-about", "About & uninstall", "version remove"],
];

function paletteItems() {
  const items = [];
  for (const g of (state.overview && state.overview.games) || []) {
    items.push({ kind: "Game", label: g.name, words: `${g.name} ${g.app_id}`, run: () => { closeSettings(); state.tab = "overview"; showGame(g.app_id); } });
  }
  const ins = insightsFor(state.selected);
  const game = state.detail && state.detail.summary;
  for (const sug of (ins && ins.suggestions) || []) {
    items.push({ kind: "Fix", label: `${sug.title}${game ? ` (${game.name})` : ""}`, words: `fix ${sug.title}`, run: () => { closeSettings(); state.tab = "fixes"; renderMain(); } });
  }
  for (const [id, label, words] of SETTING_SECTIONS) {
    items.push({ kind: "Settings", label, words: `settings ${label} ${words}`, run: () => openSettings(id) });
  }
  const home = state.detail && state.detail.fix_home;
  const actions = [
    ["Library", "home all games overview", () => { closeSettings(); goHome(); }],
    ...(state.selected && home && !home.in_steam && isWorking(state.selected)
      ? [[`Keep this fix in Steam (${game.name})`, "launch options without cachyplay", () => { closeSettings(); keepInSteam(state.selected, true); }]] : []),
    ["Report a problem", "bug report send", () => openReport({ appId: state.selected })],
    ["Check for updates", "update new version", () => { openSettings("set-updates"); checkUpdates(false); }],
    ["Run the PC check", "problems", () => { openSettings("set-pc"); runPcCheck(); }],
    ["Close Steam for me", "steam exit quit", closeSteam],
    ["Open Steam", "steam start", openSteam],
    ["Show the tour", "help tour guide", startTour],
    ["Keyboard shortcuts", "keys help", openShortcuts],
    ["What's new", "changes release notes", openWhatsNew],
    ["About CachyPlay", "version", openAbout],
    ["Quit CachyPlay", "exit close", () => invoke("quit_app")],
  ];
  for (const [label, words, run] of actions) items.push({ kind: "Action", label, words: `${label} ${words}`, run });
  return items;
}

function openPalette() {
  closeMenus();
  const dlg = $("palette");
  if (dlg.open) return;
  state.palette = { items: paletteItems(), shown: [], index: 0 };
  $("palette-input").value = "";
  renderPalette();
  dlg.showModal();
  $("palette-input").focus();
}

/* Matches in order: the name typed exactly, then names that start with it, then names with a word
   that starts with it, then everything else that matches. Within each, games, fixes, settings and
   actions keep their usual order. So "report" puts "Report a problem" first even when a game's name
   contains the word. */
function rankPalette(items, query) {
  const q = query.trim().toLowerCase();
  const words = q.split(/\s+/).filter(Boolean);
  const score = (it) => {
    const l = it.label.toLowerCase();
    if (!q) return 3;
    if (l === q) return 0;
    if (l.startsWith(q)) return 1;
    if (l.split(/[^a-z0-9]+/).some((w) => w.startsWith(words[0]))) return 2;
    return 3;
  };
  return items
    .map((it, i) => ({ it, i }))
    .filter(({ it }) => words.every((w) => it.words.toLowerCase().includes(w)))
    .sort((a, b) => score(a.it) - score(b.it) || a.i - b.i)
    .map(({ it }) => it);
}

function renderPalette() {
  const p = state.palette;
  p.shown = rankPalette(p.items, $("palette-input").value).slice(0, 12);
  p.index = Math.min(p.index, Math.max(0, p.shown.length - 1));
  const list = $("palette-list");
  if (!p.shown.length) {
    list.replaceChildren(h("li", { class: "empty-row" }, "Nothing matches. Try a game's name, a setting, or words like \"update\" or \"report\"."));
    return;
  }
  list.replaceChildren(...p.shown.map((it, i) => h("li", {
    role: "option", "aria-selected": i === p.index ? "true" : "false",
    onclick: () => runPaletteItem(i), onmousemove: () => { if (p.index !== i) { p.index = i; renderPalette(); } },
  }, h("span", { class: "palette-kind" }, it.kind), h("span", {}, it.label))));
}

function runPaletteItem(i) {
  const it = state.palette.shown[i];
  $("palette").close();
  if (it) it.run();
}

function paletteKeys(e) {
  const p = state.palette;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    p.index = (p.index + (e.key === "ArrowDown" ? 1 : -1) + p.shown.length) % Math.max(1, p.shown.length);
    renderPalette();
  } else if (e.key === "Enter") {
    e.preventDefault();
    runPaletteItem(p.index);
  }
}

/* ---------- first-run tour (new installs only; replay from Help) ---------- */
const TOUR = [
  ["#rail", "Your games", "Grouped as Needs you, Testing a fix and Working. Green runs fine (with or without a fix), amber is testing a fix, red isn't working, grey not played lately. Library at the top shows everything at a glance."],
  [".switch-card", "The CachyPlay switch", "Turns CachyPlay on or off for this game. It's the one place to do that, and turning it off puts Steam's launch options back."],
  [".brief", "What happened, why, what to do", "For the game you pick: what happened, why, and the next step, with one main button. When a game works with a fix, this shows where the fix lives and can keep it in Steam."],
  ['#tab-fixes', "The Fixes tab", "Every suggestion, ruled-out tries, and what's applied right now with an Undo button for each."],
  ["#open-settings", "Settings", "Updates, the PC check, appearance and privacy. Help (?) has bug reports, shortcuts and this tour. Press Ctrl+K any time to search everything."],
];
state.tourStep = -1;

async function startTour() {
  closeMenus();
  closeSettings();
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  // Steps 2 to 4 point at a game's page, so the tour opens one (the window starts on Library).
  const games = (state.overview && state.overview.games) || [];
  if (!state.selected && games.length) {
    const pick = games.find(needsAttention) || games.find((g) => g.enabled) || games[0];
    state.tab = "overview";
    await showGame(pick.app_id);
  }
  state.tourStep = 0;
  showTourStep();
}

function showTourStep() {
  document.querySelectorAll(".tour-target").forEach((el) => el.classList.remove("tour-target"));
  const [sel, title, text] = TOUR[state.tourStep];
  const box = $("tour");
  $("tour-count").textContent = `Step ${state.tourStep + 1} of ${TOUR.length}`;
  $("tour-title").textContent = title;
  $("tour-text").textContent = text;
  $("tour-back").hidden = state.tourStep === 0;
  $("tour-next").textContent = state.tourStep === TOUR.length - 1 ? "Done" : "Next";
  box.hidden = false;
  const target = document.querySelector(sel);
  if (target) {
    target.classList.add("tour-target");
    target.scrollIntoView({ block: "nearest" });
    const r = target.getBoundingClientRect();
    const w = box.offsetWidth, hgt = box.offsetHeight;
    let left = Math.min(Math.max(16, r.left), window.innerWidth - w - 16);
    let top = r.bottom + 12;
    if (top + hgt > window.innerHeight - 16) top = Math.max(16, r.top - hgt - 12);
    if (r.width > window.innerWidth * 0.2 && r.height > window.innerHeight * 0.5) {
      left = Math.min(r.right + 16, window.innerWidth - w - 16);
      top = Math.max(16, r.top + 40);
    }
    box.style.left = `${left}px`;
    box.style.top = `${top}px`;
  } else {
    box.style.left = `${(window.innerWidth - box.offsetWidth) / 2}px`;
    box.style.top = `${(window.innerHeight - box.offsetHeight) / 2}px`;
  }
  $("tour-next").focus();
}

function tourMove(step) {
  const next = state.tourStep + step;
  if (next >= TOUR.length) return endTour();
  state.tourStep = Math.max(0, next);
  showTourStep();
}

function endTour() {
  $("tour").hidden = true;
  document.querySelectorAll(".tour-target").forEach((el) => el.classList.remove("tour-target"));
  state.tourStep = -1;
  if (!state.settings.tour_seen) setAppSetting("tour_seen", true);
}

/* ---------- What's new and About ---------- */
async function openWhatsNew() {
  let md = "";
  try {
    md = await invoke("whats_new");
  } catch (e) {
    md = String(e);
  }
  $("whatsnew-title").textContent = `What's new in ${(state.appInfo && state.appInfo.version) || "CachyPlay"}`;
  $("whatsnew-notes").replaceChildren(...notesView(md));
  $("whatsnew-dialog").showModal();
}

function openAbout() {
  const o = state.overview || {};
  const u = state.update || {};
  $("about-version").textContent = `Version ${(state.appInfo && state.appInfo.version) || o.version || ""}${u.channel ? ` · ${CHANNEL_NAME[u.channel]} channel` : ""}`;
  $("about-facts").replaceChildren(
    h("dt", {}, "Installed at"), h("dd", {}, o.installed_cli || "Not installed yet"),
    h("dt", {}, "Licence"), h("dd", {}, "GPL-3.0-or-later (free software: you may share and change it)"),
    h("dt", {}, "Source"), h("dd", {}, `github.com/${(state.appInfo && state.appInfo.repo) || "Matt-B-007/CachyPlay"}`),
    h("dt", {}, "Fonts and icons"), h("dd", {}, "Inter and JetBrains Mono (SIL OFL), Lucide (ISC)"));
  $("about-dialog").showModal();
}

/* ---------- uninstall (House Standard dialog) ---------- */
function openUninstall() {
  $("uninstall-ask").hidden = false;
  $("uninstall-done").hidden = true;
  $("un-delete-settings").checked = false;
  $("un-delete-backups").checked = false;
  $("un-keep-settings").checked = true;
  $("uninstall-go").disabled = false;
  $("uninstall-go").textContent = "Uninstall";
  $("uninstall-dialog").showModal();
  $("uninstall-cancel").focus();
}

async function uninstallNow() {
  const go = $("uninstall-go");
  go.disabled = true;
  go.textContent = "Uninstalling…";
  try {
    const msg = await invoke("uninstall", {
      removeBackups: $("un-delete-backups").checked,
      keepSettings: $("un-keep-settings").checked,
      deleteSettings: $("un-delete-settings").checked,
    });
    $("uninstall-result").textContent = msg;
    $("uninstall-ask").hidden = true;
    $("uninstall-done").hidden = false;
    $("uninstall-close").focus();
  } catch (e) {
    go.disabled = false;
    go.textContent = "Uninstall";
    toast(`Couldn't uninstall: ${e}`, "error");
  }
}

/* ---------- start-up: tour for new installs, "what's new" after an update ---------- */
async function afterStart() {
  try {
    state.appInfo = await invoke("app_info");
  } catch {
    state.appInfo = {};
  }
  if (state.appInfo.new_install && !state.settings.tour_seen) {
    setTimeout(startTour, 600);
  } else if (state.appInfo.updated_from) {
    toast(`CachyPlay was updated to ${state.appInfo.version}.`, "success", { label: "What's new", run: openWhatsNew });
  }
}
