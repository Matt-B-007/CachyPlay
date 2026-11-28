// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// The basics: talking to CachyPlay, state, the DOM helper, messages, loading and the top bar.

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const state = {
  overview: null,
  selected: null,
  query: "",
  covers: new Map(),
  busy: false,
  update: null,
};

const BADGE_TEXT = {
  ready: "Ready",
  fix_available: "Fix available",
  your_tweaks: "Your tweaks",
  last_failed: "Last start failed",
  crashed: "Crashed recently",
  off: "Not set up",
};

/* ---------- tiny DOM helper (no innerHTML: game names are data) ---------- */
function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(possessives(String(kid))));
  }
  return el;
}

/* "Moon Sweepers's fix" reads better as "Moon Sweepers' fix" (names ending in s). */
function possessives(text) {
  return text.includes("s's") ? text.replace(/\b([A-Z0-9][\w-]*s)'s(?![\w])/g, "$1'") : text;
}

const $ = (id) => document.getElementById(id);

/* ---------- toasts ---------- */
function toast(message, kind = "info", action) {
  // When CachyPlay itself fails, offer the one useful next step.
  if (kind === "error" && !action) action = { label: "Report", run: () => openReport({ appId: state.selected, problem: message }) };
  const box = $("toasts");
  placeToasts();
  // An older message's button (Undo, Close Steam) belongs to a moment that has passed.
  for (const old of box.querySelectorAll(".toast")) {
    if (old.querySelector("button")) old.remove();
  }
  while (box.children.length >= 3) box.firstElementChild.remove();
  const t = h("div", { class: `toast ${kind}` }, h("p", {}, message));
  if (action) t.append(h("button", { class: "secondary", onclick: () => { t.remove(); action.run(); } }, action.label));
  box.append(t);
  setTimeout(() => t.remove(), action ? 12000 : 6000);
  return t;
}

/* Messages show above an open dialog (a dialog covers everything outside itself). */
function placeToasts() {
  const open = [...document.querySelectorAll("dialog[open]")].pop();
  const host = open || document.body;
  if ($("toasts").parentElement !== host) host.append($("toasts"));
}
/* ---------- loading ---------- */
state.tab = "overview";
state.detail = null;
state.insights = new Map();
state.protondb = new Map();
state.settings = {};

async function load() {
  try {
    state.overview = await invoke("overview");
  } catch (e) {
    state.overview = { version: "", games: [], steam: null, steam_error: String(e), installed_cli: null, system_crashes: null, system: {} };
  }
  renderChrome();
  renderRail();
  const games = state.overview.games;
  // The window opens on the library home; a game opens when you pick it.
  if (state.selected && !games.some((g) => g.app_id === state.selected)) state.selected = null;
  if (state.selected) showGame(state.selected, { keepScroll: true });
  else renderEmptyMain();
}

function editBlocked() {
  const o = state.overview;
  if (!o.steam) return o.steam_error || "Steam wasn't found.";
  return o.steam.blocked_reason;
}

// Old crashes don't put a game that now works (green) or is testing a fix (amber) back on top.
const needsAttention = (g) => g.light === "red" || ((g.gpu_crashes > 0 || g.last_failed) && !["yellow", "green"].includes(g.light));

function times(n) {
  return n === 1 ? "once" : `${n} times`;
}

const LIGHT_TEXT = { green: "Runs fine", yellow: "Trying a fix", red: "Not working", grey: "Not played lately" };

function statusText(g) {
  if (g.waiting) return "Waiting for Steam to close";
  if (g.stage && g.stage !== "Not working") return g.stage;
  if (g.light === "yellow") return "Trying a fix";
  if (g.gpu_crashes > 0) return `Crashed ${times(g.gpu_crashes)}`;
  if (g.light === "red") return "Not working";
  if (g.light === "green" || g.light === "yellow") return LIGHT_TEXT[g.light];
  return BADGE_TEXT[g.badge];
}

/* ---------- top bar ---------- */
function renderChrome() {
  const o = state.overview;
  $("version").textContent = o.version ? `v${o.version}` : "";
  // Driver and kernel, so it's clear what a crash happened on (the full picture is on Library).
  const sys = o.system || {};
  const sysBits = [sys.driver, sys.kernel ? `Kernel ${sys.kernel}` : null].filter(Boolean);
  $("sys-chip").hidden = !sysBits.length;
  $("sys-chip").replaceChildren(icon("monitor", 14), h("span", {}, sysBits.join(" · ")));
  $("sys-chip").title = [sys.gpu, sys.driver, sys.kernel ? `Linux ${sys.kernel}` : null, sys.distro].filter(Boolean).join(", ");
  const chip = $("steam-chip");
  // Green: Steam is open and CachyPlay can read it. Grey: closed (nothing wrong; every change can
  // be made now). Amber: closing, or changes are waiting for it to close. Red: Steam not found.
  chip.hidden = false;
  if (o.steam) {
    const st = (state.live && state.live.steam) || o.steam.state || (o.steam.running ? "running" : "closed");
    const waiting = ((state.live && state.live.waiting) || []).length;
    const kind = st === "closing" || (st === "running" && waiting) ? "busy" : st === "running" ? "open" : "closed";
    chip.className = `steam-chip ${kind}`;
    chip.title = kind === "closed"
      ? "Steam is closed: CachyPlay can make every change right away."
      : kind === "busy" ? "Changes are waiting for Steam to close." : "Steam is open and CachyPlay can read it.";
    chip.replaceChildren(
      h("span", { class: "dot" }),
      h("span", {}, st === "closing" ? "Steam is closing…" : st === "running"
        ? (waiting ? `Steam is open · ${waiting} waiting` : "Steam is open") : "Steam is closed"),
      h("span", { class: "who" }, o.steam.persona || `account ${o.steam.account_id}`)
    );
  } else {
    chip.className = "steam-chip missing";
    chip.title = o.steam_error || "Install Steam and log in once.";
    chip.replaceChildren(h("span", { class: "dot" }), h("span", {}, "Steam not found"));
  }
}

/* Banners that apply to the whole app, shown above the game. */
function banners() {
  const o = state.overview;
  const out = [];
  if (!o.installed_cli) {
    out.push(h("div", { class: "banner info" },
      h("p", {}, h("strong", {}, "Install CachyPlay before turning games on"),
        "Steam needs to find CachyPlay in a fixed place, not in your Downloads folder."),
      h("button", { class: "primary", onclick: installSelf }, "Install")));
  }
  if (!o.steam) {
    out.push(h("div", { class: "banner bad" },
      h("p", {}, h("strong", {}, "Steam wasn't found"), o.steam_error || "Install Steam and log in once.")));
  } else if (o.steam.blocked_reason) {
    const waiting = (state.live && state.live.waiting) || [];
    out.push(h("div", { class: `banner ${o.steam.kind === "flatpak" ? "bad" : "warn"}` },
      h("p", {}, h("strong", {}, o.steam.kind === "flatpak" ? "Flatpak Steam" : o.steam.running ? (waiting.length ? `${waiting.length === 1 ? "1 change is" : waiting.length + " changes are"} waiting for Steam to close` : "Steam is open") : "Read-only for now"), o.steam.blocked_reason),
      o.steam.can_queue ? h("button", { class: waiting.length ? "primary" : "secondary", onclick: closeSteam }, "Close Steam for me") : null));
  }
  const problems = (state.pcCheck || []).filter((c) => c.level === "bad");
  if (problems.length) {
    out.push(h("div", { class: "banner bad" },
      h("p", {}, h("strong", {}, problems.length === 1 ? problems[0].title : `The PC check found ${problems.length} problems`),
        problems.length === 1 ? problems[0].detail : problems.map((c) => c.title).join(", ") + "."),
      h("button", { class: "secondary", onclick: () => { openSettings(); setTimeout(() => $("set-pc").scrollIntoView({ block: "start" }), 50); } }, "See how to fix")));
  }
  if (state.lastCrash) {
    out.push(h("div", { class: "banner warn" },
      h("p", {}, h("strong", {}, "CachyPlay closed unexpectedly last time"),
        state.lastCrash.details ? "A crash report was saved. Sending it helps fix the problem." : "It may have been closed by the system or a power cut. A bug report helps if it keeps happening."),
      h("button", { class: "secondary", onclick: () => { const c = state.lastCrash; dismissCrash(); openReport({ problem: "CachyPlay closed unexpectedly." + (c && c.details ? "" : " (No crash trace: it may have been closed by the system.)") }); } }, "Report"),
      h("button", { class: "secondary", onclick: dismissCrash }, "Dismiss")));
  }
  // Games that work again don't count any more; with fewer than two left there's no pattern.
  const sc = o.system_crashes;
  const fixed = new Set(o.games.filter((g) => g.light === "green").map((g) => g.name));
  const still = sc ? sc.programs.filter((n) => !fixed.has(n)) : [];
  const scKey = sc ? `${sc.crashes}:${still.join("|")}` : "";
  if (sc && still.length >= 2 && state.dismissedSystem !== scKey) {
    out.push(h("div", { class: "banner warn system" },
      h("p", {},
        h("strong", {}, still.length === sc.programs.length
          ? `${sc.crashes} graphics driver crashes in ${still.length} different games`
          : `Graphics driver crashes in ${still.length} different games`),
        `${still.join(", ")}. When several games crash the same way, the cause is usually the driver or an add-on every game loads, not the games.`),
      h("button", { class: "secondary", title: "Hide this until something changes", onclick: () => { state.dismissedSystem = scKey; redrawBanners(); } }, "Dismiss")));
  }
  // Every banner leads with its status icon, so an amber warning never reads as the accent.
  for (const b of out) {
    const kind = b.classList.contains("info") ? "info" : b.classList.contains("bad") ? "octagon" : "alert";
    b.prepend(icon(kind, 18));
  }
  return out;
}
