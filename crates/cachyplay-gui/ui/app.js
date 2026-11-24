"use strict";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const state = {
  overview: null,
  selected: null,
  filter: "all",
  query: "",
  covers: new Map(),
  busy: false,
};

const BADGE_TEXT = {
  ready: "Ready",
  fix_available: "Fix available",
  your_tweaks: "Your tweaks",
  last_failed: "Last launch failed",
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
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const $ = (id) => document.getElementById(id);

/* ---------- toasts ---------- */
function toast(message, kind = "info", action) {
  const t = h("div", { class: `toast ${kind}` }, h("p", {}, message));
  if (action) t.append(h("button", { class: "secondary", onclick: action.run }, action.label));
  $("toasts").append(t);
  setTimeout(() => t.remove(), action ? 12000 : 6000);
  return t;
}

/* ---------- loading ---------- */
async function load() {
  try {
    state.overview = await invoke("overview");
  } catch (e) {
    state.overview = { version: "", games: [], steam: null, steam_error: String(e), installed_cli: null };
  }
  render();
  if (state.selected) showDetail(state.selected, { keepScroll: true });
}

function editBlocked() {
  const o = state.overview;
  if (!o.steam) return o.steam_error || "Steam wasn't found.";
  return o.steam.blocked_reason;
}

/* ---------- top bar + banners ---------- */
function renderChrome() {
  const o = state.overview;
  $("version").textContent = o.version ? `v${o.version}` : "";
  const chip = $("steam-chip");
  if (o.steam) {
    chip.hidden = false;
    chip.className = `steam-chip${o.steam.running ? " running" : ""}`;
    chip.replaceChildren(
      h("span", { class: "dot" }),
      h("span", {}, o.steam.running ? "Steam is open" : "Steam is closed"),
      h("span", { class: "who" }, o.steam.persona || `account ${o.steam.account_id}`)
    );
  } else {
    chip.hidden = true;
  }

  const banners = $("banners");
  banners.replaceChildren();
  if (!o.installed_cli) {
    banners.append(
      h("div", { class: "banner info" },
        h("p", {},
          h("strong", {}, "Install CachyPlay before turning games on"),
          "Steam needs to find CachyPlay in a fixed place, not in your Downloads folder."),
        h("button", { class: "primary", onclick: installSelf }, "Install"))
    );
  }
  if (!o.steam) {
    banners.append(h("div", { class: "banner bad" },
      h("p", {}, h("strong", {}, "Steam wasn't found"), o.steam_error || "Install Steam and log in once, then press Refresh.")));
  } else if (o.steam.blocked_reason) {
    banners.append(h("div", { class: `banner ${o.steam.kind === "flatpak" ? "bad" : "warn"}` },
      h("p", {}, h("strong", {}, o.steam.kind === "flatpak" ? "Flatpak Steam" : "Steam is open"), o.steam.blocked_reason)));
  }
}

/* ---------- library grid ---------- */
function matches(g) {
  if (state.query && !g.name.toLowerCase().includes(state.query)) return false;
  switch (state.filter) {
    case "on": return g.enabled;
    case "fix": return g.has_fix && !g.enabled;
    case "attention": return g.last_failed;
    default: return true;
  }
}

function initials(name) {
  return name.length > 28 ? name.slice(0, 26) + "…" : name;
}

function renderGrid() {
  const games = state.overview.games;
  const shown = games.filter(matches);
  $("library-title").textContent = games.length ? `Your games (${games.length})` : "Your games";
  const grid = $("grid");
  grid.replaceChildren(...shown.map(cardFor));
  const empty = $("grid-empty");
  if (!games.length) {
    empty.hidden = false;
    empty.textContent = state.overview.steam
      ? "No installed Steam games found. Install a game in Steam, then press Refresh."
      : "Once Steam is found, your installed games appear here.";
  } else if (!shown.length) {
    empty.hidden = false;
    empty.textContent = "No games match this filter.";
  } else {
    empty.hidden = true;
  }
}

function cardFor(g) {
  const img = h("img", { alt: "", loading: "lazy", hidden: true });
  const cover = h("div", { class: "cover" }, h("span", { class: "initials" }, initials(g.name)), img);
  loadCover(g.app_id).then((url) => {
    if (!url) return;
    img.src = url;
    img.hidden = false;
    cover.querySelector(".initials").remove();
  });
  return h("button", {
    class: "card",
    "data-app": g.app_id,
    "aria-current": state.selected === g.app_id ? "true" : "false",
    onclick: () => showDetail(g.app_id),
  },
    cover,
    h("span", { class: "card-name" }, g.name),
    h("span", { class: `badge ${g.badge}` }, BADGE_TEXT[g.badge])
  );
}

async function loadCover(appId) {
  if (state.covers.has(appId)) return state.covers.get(appId);
  const p = invoke("cover", { appId }).catch(() => null);
  state.covers.set(appId, p);
  return p;
}

function render() {
  renderChrome();
  renderGrid();
}

/* ---------- detail panel ---------- */
async function showDetail(appId, opts = {}) {
  state.selected = appId;
  for (const c of document.querySelectorAll(".card")) {
    c.setAttribute("aria-current", c.dataset.app === appId ? "true" : "false");
  }
  const panel = $("detail");
  const scroll = panel.scrollTop;
  let d;
  try {
    d = await invoke("game_detail", { appId });
  } catch (e) {
    panel.replaceChildren(h("div", { class: "detail-empty" }, h("p", {}, String(e))));
    return;
  }
  panel.replaceChildren(...detailView(d));
  if (opts.keepScroll) panel.scrollTop = scroll;
  loadCover(appId).then((url) => {
    const art = panel.querySelector(".hero-art");
    if (url && art) art.style.backgroundImage = `url("${url}")`;
  });
}

function detailView(d) {
  const s = d.summary;
  const blocked = editBlocked();
  const notInstalled = !state.overview.installed_cli;
  const canToggle = !blocked && !(notInstalled && !s.enabled);
  const why = blocked || (notInstalled && !s.enabled ? "Install CachyPlay first (button at the top)." : null);

  const toggle = h("button", {
    class: "switch",
    role: "switch",
    "aria-checked": s.enabled ? "true" : "false",
    "aria-label": `CachyPlay for ${s.name}`,
    disabled: !canToggle || state.busy,
    onclick: () => setEnabled(s.app_id, !s.enabled),
  });

  const hero = h("div", { class: "hero" },
    h("div", { class: "hero-art" }),
    h("span", { class: `badge ${s.badge}` }, BADGE_TEXT[s.badge]),
    h("h2", {}, s.name),
    h("div", { class: "switch-row" },
      h("div", { class: "label" },
        h("strong", {}, s.enabled ? "CachyPlay is on" : "CachyPlay is off"),
        h("span", {}, why || (s.enabled
          ? "Start the game from Steam as usual."
          : "Turn on to apply this game's fixes and notice crashes."))),
      toggle)
  );

  const parts = [hero];

  if (s.last_failed && d.launches[0]) {
    parts.push(h("div", { class: "crash" },
      h("strong", {}, `The last launch closed after ${Math.round(d.launches[0].runtime_secs)} seconds`),
      h("p", {}, "Save a bug report to see what went wrong. It lists any known crash patterns CachyPlay found in the logs."),
      h("button", { class: "secondary", onclick: () => saveReport(s.app_id) }, "Save bug report")));
  }

  parts.push(h("div", { class: "section" },
    h("h3", {}, "Launch path"),
    h("p", { class: "hint" }, "Where a problem would show up, from the game to your screen."),
    h("div", { class: "path" }, d.pipeline.map((n) =>
      h("div", { class: `node ${n.state}` }, h("span", { class: "ring" }), h("strong", {}, n.label), h("span", {}, n.detail))))
  ));

  const rows = d.changes.flatMap((c) => [
    h("tr", { class: `main ${c.status}` },
      h("td", { class: "setting" }, h("strong", {}, c.setting)),
      h("td", { class: "stock" }, c.stock),
      h("td", { class: "new" }, c.cachyplay)),
    h("tr", { class: `sub ${c.status}` },
      h("td", { colspan: "3" },
        h("p", {}, c.why),
        c.technical ? h("code", {}, c.technical) : null)),
  ]);
  const changes = h("div", { class: "section" },
    h("h3", {}, s.enabled ? "What CachyPlay changes" : "What CachyPlay would change"),
    h("table", { class: "changes" },
      h("thead", {}, h("tr", {}, h("th", {}, "Setting"), h("th", {}, "Stock"), h("th", {}, "CachyPlay"))),
      h("tbody", {}, rows)));
  if (d.fix && !d.fix.tested) {
    changes.append(h("p", { class: "note untested" },
      "This game's fix file hasn't been tested on real hardware yet. It only uses general, low-risk settings."));
  } else if (!d.fix) {
    changes.append(h("p", { class: "note" },
      "No fix file for this game yet. CachyPlay still records each launch and helps with bug reports."));
  }
  if (d.fix && d.fix.notes) changes.append(h("p", { class: "note" }, d.fix.notes));
  parts.push(changes);

  const userBits = [];
  if (d.user_options.trim()) userBits.push(`Launch options: ${d.user_options}`);
  userBits.push(...d.tweaks);
  if (userBits.length || d.edited_since) {
    parts.push(h("div", { class: "section" },
      h("h3", {}, "Your own tweaks"),
      h("p", { class: "hint" }, "CachyPlay keeps these and works around them."),
      h("ul", { class: "tweaks" }, userBits.map((t) => h("li", {}, t))),
      d.edited_since ? h("p", { class: "note" }, "You edited this game's launch options in Steam after CachyPlay did. Turning CachyPlay off keeps your edits.") : null));
  }

  parts.push(h("div", { class: "section" },
    h("h3", {}, "Recent launches"),
    d.launches.length
      ? h("ul", { class: "launches" }, d.launches.map((l) => h("li", {},
          h("span", { class: "when" }, when(l.started_at)),
          h("span", {}, duration(l.runtime_secs)),
          l.failed ? h("span", { class: "bad" }, "Closed early") : h("span", { class: "good" }, "Ran normally"))))
      : h("p", { class: "hint" }, s.enabled ? "Start the game from Steam and it will show up here." : "Launches show up here once CachyPlay is on.")));

  parts.push(h("div", { class: "section" },
    h("div", { class: "actions" },
      h("button", { class: "secondary", onclick: () => saveReport(s.app_id) }, "Save bug report")),
    h("details", { class: "raw" },
      h("summary", {}, "Show Steam launch options"),
      h("code", {}, d.launch_options || "(empty)"))));

  return parts;
}

function when(iso) {
  const t = new Date(iso);
  if (isNaN(t)) return iso;
  return t.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

function duration(secs) {
  if (secs < 90) return `${Math.round(secs)} s`;
  if (secs < 5400) return `${Math.round(secs / 60)} min`;
  return `${(secs / 3600).toFixed(1)} h`;
}

/* ---------- actions ---------- */
async function setEnabled(appId, on) {
  state.busy = true;
  try {
    const msg = await invoke("set_enabled", { appId, enabled: on });
    toast(msg, "success");
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    await load();
  }
}

async function saveReport(appId) {
  const progress = toast("Saving bug report…");
  try {
    const r = await invoke("create_report", { appId });
    toast(`Saved ${r.name}`, "success", { label: "Show in folder", run: () => invoke("open_folder", { path: r.folder }) });
  } catch (e) {
    toast(`Couldn't save the bug report: ${e}`, "error");
  } finally {
    progress.remove();
  }
}

async function installSelf() {
  try {
    toast(await invoke("install_self"), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  load();
}

function confirmDialog(title, text, okLabel) {
  return new Promise((resolve) => {
    const dlg = $("confirm");
    $("confirm-title").textContent = title;
    $("confirm-text").textContent = text;
    $("confirm-ok").textContent = okLabel;
    dlg.onclose = () => resolve(dlg.returnValue === "ok");
    dlg.showModal();
  });
}

function openSettings() {
  const o = state.overview;
  const facts = $("settings-facts");
  facts.replaceChildren(
    h("dt", {}, "Version"), h("dd", {}, o.version),
    h("dt", {}, "Installed at"), h("dd", {}, o.installed_cli || "Not installed yet"),
    h("dt", {}, "Steam"), h("dd", {}, o.steam ? o.steam.root : "Not found"),
    h("dt", {}, "Games turned on"), h("dd", {}, String(o.games.filter((g) => g.enabled).length)));
  const blocked = editBlocked();
  $("restore-all").disabled = !!blocked;
  $("uninstall").disabled = !!blocked;
  $("restore-all").title = blocked || "";
  $("uninstall").title = blocked || "";
  $("settings").showModal();
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

async function uninstall() {
  const keep = $("keep-backups").checked;
  const ok = await confirmDialog("Uninstall CachyPlay?",
    `Every game is restored first. ${keep ? "Backups are kept." : "Backups are deleted too."} Bug reports on your Desktop stay.`,
    "Uninstall CachyPlay");
  if (!ok) return;
  try {
    const msg = await invoke("uninstall", { removeBackups: !keep });
    $("settings").close();
    toast(msg, "success");
  } catch (e) {
    toast(String(e), "error");
  }
  load();
}

/* ---------- keyboard: arrow keys move across the grid (gamepad-ready later) ---------- */
function gridKeys(e) {
  if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
  const cards = [...document.querySelectorAll(".card")];
  const i = cards.indexOf(document.activeElement);
  if (i < 0) return;
  const top = cards[0].offsetTop;
  const cols = Math.max(1, cards.filter((c) => c.offsetTop === top).length);
  const next = { ArrowLeft: i - 1, ArrowRight: i + 1, ArrowUp: i - cols, ArrowDown: i + cols }[e.key];
  if (cards[next]) {
    e.preventDefault();
    cards[next].focus();
  }
}

/* ---------- wiring ---------- */
$("refresh").addEventListener("click", load);
$("open-settings").addEventListener("click", openSettings);
$("restore-all").addEventListener("click", restoreAll);
$("uninstall").addEventListener("click", uninstall);
$("grid").addEventListener("keydown", gridKeys);
$("search").addEventListener("input", (e) => { state.query = e.target.value.trim().toLowerCase(); renderGrid(); });
for (const b of document.querySelectorAll(".filters button")) {
  b.addEventListener("click", () => {
    state.filter = b.dataset.filter;
    for (const x of document.querySelectorAll(".filters button")) x.setAttribute("aria-selected", x === b ? "true" : "false");
    renderGrid();
  });
}
// Steam may have been opened or closed while CachyPlay was in the background.
window.addEventListener("focus", load);

load();
