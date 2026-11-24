"use strict";

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
  last_failed: "Last launch failed",
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
state.tab = "overview";
state.detail = null;
state.suggestions = new Map();

async function load() {
  try {
    state.overview = await invoke("overview");
  } catch (e) {
    state.overview = { version: "", games: [], steam: null, steam_error: String(e), installed_cli: null, system_crashes: null };
  }
  renderChrome();
  renderRail();
  const games = state.overview.games;
  if (!state.selected || !games.some((g) => g.app_id === state.selected)) {
    const first = [...games].filter(needsAttention).sort((a, b) => b.gpu_crashes - a.gpu_crashes)[0] || games[0];
    state.selected = first ? first.app_id : null;
  }
  if (state.selected) showGame(state.selected, { keepScroll: true });
  else renderEmptyMain();
}

function editBlocked() {
  const o = state.overview;
  if (!o.steam) return o.steam_error || "Steam wasn't found.";
  return o.steam.blocked_reason;
}

const needsAttention = (g) => g.gpu_crashes > 0 || g.last_failed;

function times(n) {
  return n === 1 ? "once" : `${n} times`;
}

function statusText(g) {
  if (g.gpu_crashes > 0) return `Crashed ${times(g.gpu_crashes)}`;
  return BADGE_TEXT[g.badge];
}

/* ---------- top bar ---------- */
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
    out.push(h("div", { class: `banner ${o.steam.kind === "flatpak" ? "bad" : "warn"}` },
      h("p", {}, h("strong", {}, o.steam.kind === "flatpak" ? "Flatpak Steam" : "Steam is open"), o.steam.blocked_reason)));
  }
  const sc = o.system_crashes;
  if (sc) {
    out.push(h("div", { class: "banner warn system" },
      h("p", {},
        h("strong", {}, `${sc.crashes} graphics driver crashes in ${sc.programs.length} different games`),
        `${sc.programs.join(", ")}. When several games crash the same way, the cause is usually the driver or an add-on every game loads, not the games.`)));
  }
  return out;
}

/* ---------- game rail ---------- */
function renderRail() {
  const rail = $("rail");
  const q = state.query;
  const games = state.overview.games.filter((g) => !q || g.name.toLowerCase().includes(q));
  const attention = games.filter(needsAttention).sort((a, b) => b.gpu_crashes - a.gpu_crashes);
  const rest = games.filter((g) => !needsAttention(g));
  const parts = [];
  if (attention.length) {
    parts.push(h("h2", { class: "rail-title attention" }, "Needs attention"));
    parts.push(...attention.map(railItem));
  }
  if (rest.length) {
    parts.push(h("h2", { class: "rail-title" }, attention.length ? "All games" : `Your games (${rest.length})`));
    parts.push(...rest.map(railItem));
  }
  if (!games.length) {
    parts.push(h("p", { class: "rail-empty" }, state.overview.games.length
      ? "No games match your search."
      : state.overview.steam ? "No installed Steam games found." : "Once Steam is found, your games appear here."));
  }
  rail.replaceChildren(...parts);
}

function railItem(g) {
  const thumb = h("span", { class: "thumb" }, h("span", {}, g.name.slice(0, 2).toUpperCase()));
  loadCover(g.app_id).then((url) => {
    if (!url) return;
    thumb.replaceChildren(h("img", { src: url, alt: "" }));
  });
  return h("button", {
    class: `rail-item ${g.badge}`,
    "data-app": g.app_id,
    "aria-current": state.selected === g.app_id ? "true" : "false",
    onclick: () => { state.tab = "overview"; showGame(g.app_id); },
  },
    thumb,
    h("span", { class: "rail-text" },
      h("span", { class: "rail-name" }, g.name),
      h("span", { class: "rail-status" }, statusText(g))));
}

async function loadCover(appId) {
  if (state.covers.has(appId)) return state.covers.get(appId);
  const p = invoke("cover", { appId }).catch(() => null);
  state.covers.set(appId, p);
  return p;
}

/* ---------- main area ---------- */
function renderEmptyMain() {
  $("main").replaceChildren(...banners(),
    h("div", { class: "main-empty" }, h("img", { src: "logo.svg", alt: "", width: "72", height: "72" }),
      h("p", {}, state.overview.steam ? "Install a game in Steam and it appears here." : "Your games appear here once Steam is found.")));
}

async function showGame(appId, opts = {}) {
  state.selected = appId;
  for (const c of document.querySelectorAll(".rail-item")) {
    c.setAttribute("aria-current", c.dataset.app === appId ? "true" : "false");
  }
  const main = $("main");
  const scroll = main.scrollTop;
  try {
    state.detail = await invoke("game_detail", { appId });
  } catch (e) {
    main.replaceChildren(...banners(), h("div", { class: "main-empty" }, h("p", {}, String(e))));
    return;
  }
  renderMain();
  if (opts.keepScroll) main.scrollTop = scroll;
  try {
    const list = await invoke("game_suggestions", { appId });
    state.suggestions.set(appId, list);
  } catch {
    state.suggestions.set(appId, []);
  }
  if (state.selected === appId) renderMain();
}

function renderMain() {
  const d = state.detail;
  const main = $("main");
  const scroll = main.scrollTop;
  main.replaceChildren(...banners(), hero(d), tabs(d), h("div", { class: "tab-panel", role: "tabpanel", id: "tab-panel" }, ...panel(d)));
  main.scrollTop = scroll;
  loadCover(d.summary.app_id).then((url) => {
    const box = main.querySelector(".hero-cover");
    if (url && box) box.replaceChildren(h("img", { src: url, alt: "" }));
  });
}

function suggestionsFor(appId) {
  return state.suggestions.get(appId);
}

function topFix(list) {
  return (list || []).find((s) => s.fix && s.status === "new");
}

/* The one sentence and one main button at the top of a game. */
function hero(d) {
  const s = d.summary;
  const list = suggestionsFor(s.app_id);
  // A fix that's applied and not yet proven useless comes first: the next step is to play, not to stack another fix.
  const appliedFix = (list || []).find((x) => x.fix && (x.status === "applied" || x.status === "worked"));
  const fix = appliedFix ? null : topFix(list);
  const blocked = editBlocked();
  const notInstalled = !state.overview.installed_cli;

  let sentence;
  let accent = null;
  let primary = null;
  if (s.gpu_crashes > 0 || s.last_failed) {
    sentence = s.gpu_crashes > 0 ? `Crashed ${times(s.gpu_crashes)} recently.` : "The last launch closed early.";
    if (appliedFix) {
      accent = appliedFix.status === "worked" ? "The fix you applied is working." : "You applied a fix. Start the game from Steam to try it.";
      primary = h("button", { class: "primary big", onclick: () => switchTab("fixes") }, "See fixes");
    } else if (fix) {
      accent = (list || []).some((x) => x.status === "didnt_help") ? "Another fix is ready." : "A fix is ready.";
      primary = h("button", { class: "primary big", onclick: (e) => applyFix(s.app_id, fix.id, e.currentTarget) }, "Apply recommended fix");
    } else if (list === undefined) {
      accent = "Looking for fixes…";
    }
  } else if (s.enabled) {
    sentence = "Ready. Start it from Steam as usual.";
  } else if (s.has_fix) {
    sentence = "A fix file is available for this game.";
    primary = h("button", { class: "primary big", disabled: !!blocked || notInstalled, title: blocked || "", onclick: () => setEnabled(s.app_id, true) }, "Turn on CachyPlay");
  } else {
    sentence = "CachyPlay is off for this game.";
    primary = h("button", { class: "primary big", disabled: !!blocked || notInstalled, title: blocked || "", onclick: () => setEnabled(s.app_id, true) }, "Turn on CachyPlay");
  }

  const canToggle = !blocked && !(notInstalled && !s.enabled);
  const switchNote = blocked ? "Close Steam to change" : notInstalled && !s.enabled ? "Install CachyPlay first" : s.enabled ? "On for this game" : "Off for this game";

  return h("section", { class: "hero" },
    h("div", { class: "hero-cover" }, h("span", {}, s.name.slice(0, 2).toUpperCase())),
    h("div", { class: "hero-text" },
      h("h1", {}, s.name),
      h("p", { class: "sentence" }, sentence, accent ? " " : null, accent ? h("span", { class: "accent" }, accent) : null),
      h("div", { class: "hero-actions" },
        primary,
        h("button", { class: primary ? "secondary big" : "primary big", onclick: () => saveReport(s.app_id) }, "Save bug report"))),
    h("div", { class: "switch-card" },
      h("span", { class: "switch-label" }, h("strong", {}, "CachyPlay"), h("span", {}, switchNote)),
      h("button", {
        class: "switch",
        role: "switch",
        "aria-checked": s.enabled ? "true" : "false",
        "aria-label": `CachyPlay for ${s.name}`,
        disabled: !canToggle || state.busy,
        onclick: () => setEnabled(s.app_id, !s.enabled),
      })));
}

const TABS = [["overview", "Overview"], ["fixes", "Fixes"], ["settings", "Settings"], ["history", "History"]];

function tabs(d) {
  const list = suggestionsFor(d.summary.app_id);
  return h("div", { class: "tabs", role: "tablist", "aria-label": "Game sections", onkeydown: tabKeys },
    TABS.map(([id, label]) => h("button", {
      role: "tab",
      id: `tab-${id}`,
      "aria-selected": state.tab === id ? "true" : "false",
      "aria-controls": "tab-panel",
      tabindex: state.tab === id ? "0" : "-1",
      onclick: () => switchTab(id),
    }, label, id === "fixes" && list && list.length ? h("span", { class: "count" }, String(list.length)) : null)));
}

function switchTab(id) {
  state.tab = id;
  renderMain();
  const t = document.getElementById(`tab-${id}`);
  if (t) t.focus();
}

function tabKeys(e) {
  if (!["ArrowLeft", "ArrowRight"].includes(e.key)) return;
  const i = TABS.findIndex(([id]) => id === state.tab);
  const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length][0];
  e.preventDefault();
  switchTab(next);
}

function panel(d) {
  switch (state.tab) {
    case "fixes": return fixesTab(d);
    case "settings": return settingsTab(d);
    case "history": return historyTab(d);
    default: return overviewTab(d);
  }
}

function card(cls, ...kids) {
  return h("article", { class: `card ${cls || ""}` }, ...kids);
}

function overviewTab(d) {
  const s = d.summary;
  const list = suggestionsFor(s.app_id);
  let left;
  if (list === undefined) {
    left = card("", h("div", { class: "skeleton" }), h("div", { class: "skeleton short" }), h("div", { class: "skeleton" }));
  } else if (list.length) {
    const top = list[0];
    left = card("recommended",
      h("span", { class: "eyebrow" }, top.status === "new" ? "Recommended fix" : FIX_STATUS[top.status][0]),
      ...suggestionBody(s.app_id, top),
      list.length > 1 ? h("button", { class: "link", onclick: () => switchTab("fixes") }, `See all ${list.length} suggestions`) : null);
  } else {
    left = card("calm",
      h("span", { class: "eyebrow good" }, "No problems found"),
      h("h2", {}, s.enabled ? "Nothing to fix right now" : "No crashes recorded for this game"),
      h("p", { class: "muted" }, s.enabled
        ? "CachyPlay checks every launch. If something goes wrong, the fix shows up here."
        : "Turn CachyPlay on to apply this game's fixes and notice crashes as they happen."));
  }

  const path = card("",
    h("h2", { class: "card-title" }, "Launch path"),
    h("div", { class: "path" }, d.pipeline.map((n) =>
      h("div", { class: `node ${n.state}` }, h("span", { class: "ring" }), h("strong", {}, n.label), h("span", {}, n.detail)))));

  const activity = d.crashes.length
    ? card("grow",
        h("h2", { class: "card-title" }, "Recent crashes"),
        h("ul", { class: "rows" }, d.crashes.slice(0, 5).map((c) => h("li", {}, h("span", { class: "when" }, c.when), h("span", {}, c.what)))))
    : card("grow",
        h("h2", { class: "card-title" }, "Recent launches"),
        d.launches.length
          ? h("ul", { class: "rows" }, d.launches.slice(0, 4).map(launchRow))
          : h("p", { class: "muted" }, s.enabled ? "Start the game from Steam and it shows up here." : "Launches show up here once CachyPlay is on."));

  return [h("div", { class: "overview" }, left, h("div", { class: "stack" }, path, activity))];
}

function launchRow(l) {
  return h("li", {},
    h("span", { class: "when" }, when(l.started_at)),
    h("span", {}, duration(l.runtime_secs)),
    l.failed ? h("span", { class: "bad" }, "Closed early") : h("span", { class: "good" }, "Ran normally"));
}

/* ---------- suggested fixes ---------- */
const FIX_STATUS = {
  new: ["Suggested", ""],
  applied: ["Applied", "Start the game from Steam to try it."],
  worked: ["Worked", "The game has run normally since you applied this."],
  didnt_help: ["Didn't help", "The game crashed again after this. Try the next suggestion, or undo it."],
};

function suggestionBody(appId, s) {
  const out = [
    h("h2", {}, s.title),
    h("ul", { class: "why" }, s.why.map((w) => h("li", {}, w))),
  ];
  if (s.status !== "new") out.push(h("p", { class: `s-note ${s.status}` }, FIX_STATUS[s.status][1]));
  if (s.steps.length) out.push(h("ol", { class: "steps" }, s.steps.map((st) => h("li", {}, st))));
  if (s.fix) {
    const tech = [...Object.entries(s.fix.env).map(([k, v]) => `${k}=${v}`), ...s.fix.args].join(" ");
    out.push(h("code", { class: "tech" }, tech));
    out.push(h("div", { class: "actions" }, s.status !== "new"
      ? h("button", { class: "secondary", onclick: () => undoFix(appId, s.id) }, "Undo")
      : h("button", { class: "primary", onclick: (e) => applyFix(appId, s.id, e.currentTarget) }, "Apply fix")));
  }
  return out;
}

function fixesTab(d) {
  const list = suggestionsFor(d.summary.app_id);
  if (list === undefined) return [card("", h("div", { class: "skeleton" }), h("div", { class: "skeleton short" }))];
  if (!list.length) {
    return [card("calm", h("h2", {}, "Nothing to fix right now"),
      h("p", { class: "muted" }, "CachyPlay found no crashes or problems for this game."))];
  }
  return [
    h("p", { class: "muted intro" }, "Worked out on your PC from crashes, logs and your system, most likely first. Every fix can be undone."),
    h("ol", { class: "fix-list" }, list.map((s, i) => h("li", {},
      card(i === 0 && s.status === "new" ? "recommended" : "",
        h("div", { class: "fix-head" },
          h("span", { class: "num" }, String(i + 1)),
          s.status !== "new" ? h("span", { class: `pill ${s.status}` }, FIX_STATUS[s.status][0]) : null),
        ...suggestionBody(d.summary.app_id, s))))),
  ];
}

function settingsTab(d) {
  const s = d.summary;
  const rows = d.changes.flatMap((c) => [
    h("tr", { class: `main ${c.status}` },
      h("td", { class: "setting" }, h("strong", {}, c.setting)),
      h("td", { class: "stock" }, c.stock),
      h("td", { class: "new" }, c.cachyplay)),
    h("tr", { class: `sub ${c.status}` },
      h("td", { colspan: "3" }, h("p", {}, c.why), c.technical ? h("code", {}, c.technical) : null)),
  ]);
  const out = [card("",
    h("h2", { class: "card-title" }, s.enabled ? "What CachyPlay changes" : "What CachyPlay would change"),
    h("table", { class: "changes" },
      h("thead", {}, h("tr", {}, h("th", {}, "Setting"), h("th", {}, "Stock"), h("th", {}, "CachyPlay"))),
      h("tbody", {}, rows)),
    d.fix && !d.fix.tested ? h("p", { class: "note untested" }, "This game's fix file hasn't been tested on real hardware yet. It only uses general, low-risk settings.") : null,
    !d.fix ? h("p", { class: "note" }, "No fix file for this game yet. CachyPlay still records each launch and helps with bug reports.") : null,
    d.fix && d.fix.notes ? h("p", { class: "note" }, d.fix.notes) : null)];

  if (d.standalone_launch_options) {
    out.push(card("keep",
      h("h2", { class: "card-title" }, "Keep these settings without CachyPlay"),
      h("p", { class: "muted" }, "Paste this into the game's Launch options in Steam. Uninstalling with \"Keep the settings\" does this for you."),
      h("code", { class: "kept-line" }, d.standalone_launch_options),
      h("div", { class: "actions" }, h("button", { class: "secondary", onclick: () => copyText(d.standalone_launch_options) }, "Copy"))));
  }

  const userBits = [];
  if (d.user_options.trim()) userBits.push(`Launch options: ${d.user_options}`);
  userBits.push(...d.tweaks);
  if (userBits.length || d.edited_since) {
    out.push(card("",
      h("h2", { class: "card-title" }, "Your own tweaks"),
      h("p", { class: "muted" }, "CachyPlay keeps these and works around them."),
      h("ul", { class: "tweaks" }, userBits.map((t) => h("li", {}, t))),
      d.edited_since ? h("p", { class: "note" }, "You edited this game's launch options in Steam after CachyPlay did. Turning CachyPlay off keeps your edits.") : null));
  }
  out.push(h("details", { class: "raw" }, h("summary", {}, "Show Steam's launch options for this game"), h("code", {}, d.launch_options || "(empty)")));
  return out;
}

function historyTab(d) {
  return [
    card("",
      h("h2", { class: "card-title" }, "Graphics driver crashes"),
      d.crashes.length
        ? h("ul", { class: "rows" }, d.crashes.map((c) => h("li", {}, h("span", { class: "when" }, c.when), h("span", {}, c.what))))
        : h("p", { class: "muted" }, "None recorded since the last restart.")),
    card("",
      h("h2", { class: "card-title" }, "Launches through CachyPlay"),
      d.launches.length
        ? h("ul", { class: "rows" }, d.launches.map(launchRow))
        : h("p", { class: "muted" }, d.summary.enabled ? "Start the game from Steam and it shows up here." : "Launches are recorded once CachyPlay is on.")),
  ];
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
async function applyFix(appId, id, btn) {
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Applying…";
  }
  try {
    toast(await invoke("apply_fix", { appId, id }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.suggestions.delete(appId);
  await load();
}

async function undoFix(appId, id) {
  try {
    toast(await invoke("undo_fix", { appId, id }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.suggestions.delete(appId);
  await load();
}

async function setEnabled(appId, on) {
  state.busy = true;
  try {
    toast(await invoke("set_enabled", { appId, enabled: on }), "success");
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
    const r = await invoke("create_report", { appId: appId || null });
    toast(`Saved ${r.name}`, "success", { label: "Show in folder", run: () => invoke("open_folder", { path: r.folder }) });
  } catch (e) {
    toast(`Couldn't save the bug report: ${e}`, "error");
  } finally {
    progress.remove();
  }
  if ($("reports").open) renderReports();
}

/* ---------- bug reports screen ---------- */
async function renderReports() {
  const list = $("report-list");
  let reports = [];
  try {
    reports = await invoke("saved_reports");
  } catch (e) {
    list.replaceChildren(h("li", { class: "empty-row" }, String(e)));
    return;
  }
  if (!reports.length) {
    list.replaceChildren(h("li", { class: "empty-row" }, "No bug reports saved yet. They're saved to your Desktop."));
    return;
  }
  list.replaceChildren(...reports.map((r) => h("li", {},
    h("div", { class: "what" },
      h("strong", {}, r.game || (r.app_id ? `Game ${r.app_id}` : "General report")),
      h("span", {}, `Saved ${when(r.created.replace(" ", "T"))}, ${r.size_kb} KB`)),
    h("button", { type: "button", class: "secondary", onclick: () => invoke("open_folder", { path: r.path }) }, "Show in folder"))));
}

function openReports() {
  renderReports();
  $("reports").showModal();
}

/* ---------- updates ---------- */
const CHANNEL_NAME = { stable: "Stable", beta: "Beta" };

async function checkUpdates(silent) {
  const line = $("update-line");
  if (!silent) {
    line.className = "status-line";
    line.textContent = "Checking for updates…";
  }
  try {
    state.update = await invoke("update_status");
  } catch (e) {
    state.update = { error: String(e) };
  }
  renderUpdate();
}

function renderUpdate() {
  const u = state.update;
  const pill = $("update-pill");
  const line = $("update-line");
  if (!u) return;
  for (const r of document.querySelectorAll('input[name="channel"]')) r.checked = r.value === u.channel;
  $("roll-back").hidden = !u.can_roll_back;
  if (u.error) {
    pill.hidden = true;
    line.className = "status-line bad";
    line.textContent = u.error;
    return;
  }
  if (u.available && u.latest) {
    pill.hidden = false;
    pill.textContent = `Update available: ${u.latest.tag}`;
    line.className = "status-line new";
    line.textContent = `${u.latest.tag} is available. You have v${u.current}.`;
  } else {
    pill.hidden = true;
    line.className = "status-line good";
    line.textContent = `You have the newest ${CHANNEL_NAME[u.channel]} version (v${u.current}).`;
  }
}

function openUpdateDialog() {
  const u = state.update;
  if (!u || !u.latest) return;
  $("update-title").textContent = `CachyPlay ${u.latest.tag} is available`;
  $("update-notes").replaceChildren(...notesView(u.latest.notes || ""));
  const btn = $("update-now");
  btn.disabled = false;
  btn.textContent = "Update now";
  btn.onclick = updateNow;
  $("update-command").hidden = true;
  btn.hidden = false;
  if (u.kind === "package") {
    $("update-sub").textContent = `You have v${u.current}. CachyPlay was installed from the AUR, so update it with your package manager:`;
    $("update-command").hidden = false;
    $("update-command-text").value = u.command;
    btn.hidden = true;
  } else if (u.kind === "notinstalled") {
    $("update-sub").textContent = `You have v${u.current}, running from a download or build folder. Install CachyPlay first (button at the top), then update.`;
    btn.hidden = true;
  } else {
    $("update-sub").textContent = `You have v${u.current}. The download is checked before it's installed, and your current version is kept so you can go back.`;
  }
  $("update-dialog").showModal();
}

/* Release notes are markdown from GitHub; show headings and bullets, everything as text. */
function notesView(md) {
  const out = [];
  let list = null;
  for (const raw of md.split(/\r?\n/)) {
    const line = raw.trim();
    const bullet = line.match(/^[-*]\s+(.*)/);
    if (bullet) {
      if (!list) out.push((list = h("ul")));
      list.append(h("li", {}, bullet[1].replace(/\*\*/g, "")));
      continue;
    }
    list = null;
    if (!line) continue;
    const head = line.match(/^#{1,6}\s+(.*)/);
    out.push(head ? h("h4", {}, head[1]) : h("p", {}, line.replace(/\*\*/g, "")));
  }
  return out;
}

async function updateNow() {
  const btn = $("update-now");
  btn.disabled = true;
  btn.textContent = "Updating…";
  try {
    const msg = await invoke("install_update");
    toast(msg, "success");
    $("update-title").textContent = `Updated to ${state.update.latest.tag}`;
    $("update-sub").textContent = "Restart CachyPlay to start using the new version.";
    state.update.available = false;
    state.update.can_roll_back = true;
    renderUpdate();
    btn.disabled = false;
    btn.textContent = "Restart CachyPlay";
    btn.onclick = () => invoke("restart_app");
  } catch (e) {
    toast(`Update failed: ${e}`, "error");
    btn.disabled = false;
    btn.textContent = "Try again";
  }
}

async function copyCommand() {
  const input = $("update-command-text");
  try {
    await navigator.clipboard.writeText(input.value);
    toast("Copied. Paste it into a terminal.", "success");
  } catch {
    input.select();
    toast("Press Ctrl+C to copy the selected command.");
  }
}

async function setChannel(channel) {
  try {
    await invoke("set_channel", { channel });
  } catch (e) {
    toast(String(e), "error");
  }
  checkUpdates(false);
}

async function rollBack() {
  const ok = await confirmDialog("Go back to the previous version?", "CachyPlay will switch back to the version you had before the last update.", "Go back");
  if (!ok) return;
  try {
    const msg = await invoke("roll_back");
    toast(msg, "success", { label: "Restart now", run: () => invoke("restart_app") });
  } catch (e) {
    toast(String(e), "error");
  }
  checkUpdates(true);
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
  renderUpdate();
  if (!state.update) checkUpdates(false);
  renderKept();
  $("settings").showModal();
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied. Paste it into the game's Launch options in Steam.", "success");
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
    h("code", { class: "kept-line" }, g.launch_options),
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

async function uninstall() {
  const keep = $("keep-backups").checked;
  const keepSettings = $("keep-settings").checked;
  const first = keepSettings
    ? "Each game's working settings are written into its Steam launch options, so it keeps running the same way, and a copy is saved to your Desktop."
    : "Every game's original launch options are put back.";
  const ok = await confirmDialog("Uninstall CachyPlay?",
    `${first} ${keep ? "Backups are kept." : "Backups are deleted too."} Bug reports on your Desktop stay.`,
    "Uninstall CachyPlay");
  if (!ok) return;
  try {
    const msg = await invoke("uninstall", { removeBackups: !keep, keepSettings });
    $("settings").close();
    toast(msg, "success");
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

/* ---------- wiring ---------- */
$("open-settings").addEventListener("click", openSettings);
$("open-reports").addEventListener("click", openReports);
$("general-report").addEventListener("click", () => saveReport(null));
$("open-issues").addEventListener("click", () => invoke("open_issue_page").catch((e) => toast(String(e), "error")));
$("update-pill").addEventListener("click", openUpdateDialog);
$("check-updates").addEventListener("click", () => checkUpdates(false));
$("roll-back").addEventListener("click", rollBack);
$("copy-command").addEventListener("click", copyCommand);
$("export-settings").addEventListener("click", exportSettings);
for (const r of document.querySelectorAll('input[name="channel"]')) {
  r.addEventListener("change", () => r.checked && setChannel(r.value));
}
$("restore-all").addEventListener("click", restoreAll);
$("uninstall").addEventListener("click", uninstall);
$("rail").addEventListener("keydown", railKeys);
$("search").addEventListener("input", (e) => { state.query = e.target.value.trim().toLowerCase(); renderRail(); });

// Steam may have been opened or closed while CachyPlay was in the background.
// Suggestions are recomputed too, since a game may have crashed meanwhile.
window.addEventListener("focus", () => { state.suggestions.clear(); load(); });

load();
// Check for a newer CachyPlay once at startup, in the background; never blocks anything.
checkUpdates(true);
