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
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const $ = (id) => document.getElementById(id);

/* ---------- toasts ---------- */
function toast(message, kind = "info", action) {
  // When CachyPlay itself fails, offer the one useful next step.
  if (kind === "error" && !action) action = { label: "Save bug report", run: () => saveReport(null) };
  const t = h("div", { class: `toast ${kind}` }, h("p", {}, message));
  if (action) t.append(h("button", { class: "secondary", onclick: action.run }, action.label));
  $("toasts").append(t);
  setTimeout(() => t.remove(), action ? 12000 : 6000);
  return t;
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

const needsAttention = (g) => g.light === "red" || ((g.gpu_crashes > 0 || g.last_failed) && g.light !== "yellow");

function times(n) {
  return n === 1 ? "once" : `${n} times`;
}

const LIGHT_TEXT = { green: "Runs fine", yellow: "Works with a fix", red: "Not working", grey: "Not played lately" };

function statusText(g) {
  if (g.waiting) return "Waiting for Steam to close";
  if (g.stage && g.stage !== "Not working") return g.stage;
  if (g.light === "yellow") return g.gpu_crashes > 0 ? "Trying a fix" : "Works with a fix";
  if (g.gpu_crashes > 0) return `Crashed ${times(g.gpu_crashes)}`;
  if (g.light === "red") return "Not working";
  if (g.light === "green" || g.light === "yellow") return LIGHT_TEXT[g.light];
  return BADGE_TEXT[g.badge];
}

/* ---------- top bar ---------- */
function renderChrome() {
  const o = state.overview;
  $("version").textContent = o.version ? `v${o.version}` : "";
  const chip = $("steam-chip");
  if (o.steam) {
    chip.hidden = false;
    const st = (state.live && state.live.steam) || o.steam.state || (o.steam.running ? "running" : "closed");
    chip.className = `steam-chip${st !== "closed" ? " running" : ""}`;
    chip.replaceChildren(
      h("span", { class: "dot" }),
      h("span", {}, st === "running" ? "Steam is open" : st === "closing" ? "Steam is closing…" : "Steam is closed"),
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
    class: `rail-item ${g.badge} light-${g.light}`,
    "data-app": g.app_id,
    "aria-current": state.selected === g.app_id ? "true" : "false",
    onclick: () => { state.tab = "overview"; showGame(g.app_id); },
  },
    thumb,
    h("span", { class: "rail-text" },
      h("span", { class: "rail-name" }, g.name),
      h("span", { class: "rail-status" }, h("span", { class: `light ${g.light}`, "aria-hidden": "true" }), statusText(g))));
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
    state.insights.set(appId, await invoke("game_insights", { appId }));
  } catch (e) {
    state.insights.set(appId, { suggestions: [], summary: null, error: String(e) });
  }
  if (state.selected === appId) renderMain();
  if (state.settings.protondb && !state.protondb.has(appId)) {
    state.protondb.set(appId, null);
    invoke("protondb_rating", { appId }).then((r) => {
      state.protondb.set(appId, r);
      if (state.selected === appId) renderMain();
    }).catch(() => {});
  }
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

function insightsFor(appId) {
  return state.insights.get(appId);
}

function suggestionsFor(appId) {
  const i = insightsFor(appId);
  return i ? i.suggestions : undefined;
}

const STAGE_LABEL = { not_working: "Not working", testing: "Testing a fix", seems_to_work: "Seems to work", works_with_fix: "Works with a fix", runs_fine: "Runs fine" };

function stageLabel(d) {
  const p = insightsFor(d.summary.app_id)?.progress;
  if (!p) return null;
  if (p.stage === "testing" && p.playing_secs != null) return "Playing now";
  return STAGE_LABEL[p.stage] || null;
}

const LIGHT_LABEL = { green: "Runs fine", yellow: "Works with a fix", red: "Not working", grey: "Not played lately" };

/* The three lines at the top of a game (what happened, why, what to do) and one main button. */
function hero(d) {
  const s = d.summary;
  const ins = insightsFor(s.app_id);
  const sum = ins && ins.summary;
  const blocked = editBlocked();
  const notInstalled = !state.overview.installed_cli;
  const light = sum ? sum.light : (s.light || "grey");

  let lines;
  if (!ins) {
    lines = [h("p", { class: "what" }, "Looking at this game's history…")];
  } else {
    lines = [
      h("p", { class: "what" }, sum.what),
      sum.why ? h("p", { class: "why-line" }, sum.why) : null,
      sum.todo ? h("p", { class: "todo" }, h("span", { class: "todo-label" }, "What to do: "), sum.todo) : null,
    ];
  }
  if (s.removed_in_steam && !s.waiting) {
    lines.unshift(h("div", { class: "note put-back" },
      h("p", {}, h("strong", {}, "CachyPlay was removed from this game's launch options in Steam. "),
        "Editing launch options in Steam replaces the whole line, including CachyPlay's part. Put it back and keep your new options? Tip: use \"Try these settings\" on the Settings tab instead, so Steam's launch options never change."),
      h("div", { class: "actions" }, h("button", { class: "primary", disabled: notInstalled || (blocked && !canQueueNow()), onclick: () => setEnabled(s.app_id, true) }, canQueueNow() ? "Put CachyPlay back when Steam closes" : "Put CachyPlay back"))));
  }
  let primary = null;
  if (sum && sum.fix_id) {
    primary = h("button", { class: "primary big", disabled: state.busy, onclick: (e) => fixIt(s.app_id, e.currentTarget) }, "Fix it");
  } else if (!s.enabled && light !== "red") {
    primary = h("button", { class: "primary big", disabled: (!!blocked && !canQueueNow()) || notInstalled || !!s.waiting, title: blocked || "", onclick: () => setEnabled(s.app_id, true) }, s.waiting ? "Turns on when Steam closes" : canQueueNow() ? "Turn on when Steam closes" : "Turn on CachyPlay");
  }
  const rating = state.protondb.get(s.app_id);

  const canQueue = !!(blocked && state.overview.steam && state.overview.steam.can_queue);
  const canToggle = (!blocked || canQueue) && !(notInstalled && !s.enabled);
  const switchNote = s.waiting ? s.waiting.replace(/ for .*$/, "") + " when Steam closes"
    : canQueue ? (s.enabled ? "On · changes wait for Steam to close" : "Off · turning on waits for Steam to close")
    : blocked ? "Close Steam to change" : notInstalled && !s.enabled ? "Install CachyPlay first" : s.enabled ? "On for this game" : "Off for this game";

  return h("section", { class: "hero" },
    h("div", { class: "hero-cover" }, h("span", {}, s.name.slice(0, 2).toUpperCase())),
    h("div", { class: "hero-text" },
      h("div", { class: "hero-title" },
        h("h1", {}, s.name),
        h("span", { class: `light-chip ${light}` }, h("span", { class: `light ${light}` }), stageLabel(d) || LIGHT_LABEL[light]),
        rating ? h("button", { class: `protondb ${rating.tier}`, title: `${rating.meaning} (${rating.reports} reports on ProtonDB)`, onclick: () => openUrl(rating.url) }, `ProtonDB: ${rating.tier}`) : null),
      ...lines,
      h("div", { class: "hero-actions" },
        primary,
        h("button", { class: primary ? "secondary big" : "primary big", onclick: () => saveReport(s.app_id) }, "Save bug report"),
        sum && sum.fix_id ? h("span", { class: "fix-caption" }, `Fix it tries: ${sum.fix_label}. You can undo it.`) : null)),
    h("div", { class: "switch-card" },
      h("span", { class: "switch-label" }, h("strong", {}, "CachyPlay"), h("span", {}, switchNote)),
      h("button", {
        class: "switch",
        role: "switch",
        "aria-checked": s.enabled ? "true" : "false",
        "aria-label": `CachyPlay for ${s.name}`,
        disabled: !canToggle || state.busy,
        onclick: () => setEnabled(s.app_id, s.waiting ? s.waiting.startsWith("Turn CachyPlay off") : !s.enabled),
      }),
      s.waiting ? h("button", { class: "link small", onclick: () => cancelWaiting(s.app_id) }, "Cancel") : null));
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
  const ins = insightsFor(s.app_id);
  const list = ins ? ins.suggestions : undefined;
  const out = [];

  // "Did it work?" comes first: the answer makes everything else smarter.
  for (const x of (list || []).filter((x) => x.ask_verdict)) {
    out.push(card("ask",
      h("h2", {}, `Did "${x.title}" help?`),
      h("p", { class: "muted" }, `You've played ${s.name} since you applied it.`),
      h("div", { class: "actions" },
        h("button", { class: "primary", onclick: () => setVerdict(s.app_id, x.id, true) }, "Yes, it works now"),
        h("button", { class: "secondary", onclick: () => setVerdict(s.app_id, x.id, false) }, "No, still broken"))));
  }

  let left;
  if (list === undefined) {
    left = card("", h("div", { class: "skeleton" }), h("div", { class: "skeleton short" }), h("div", { class: "skeleton" }));
  } else if (list.length) {
    const top = list[0];
    left = card("recommended",
      h("span", { class: "eyebrow" }, top.status === "new" ? "Most likely" : FIX_STATUS[top.status][0]),
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
    h("h2", { class: "card-title" }, "How the game starts"),
    h("div", { class: "path" }, d.pipeline.map((n) =>
      h("div", { class: `node ${n.state}` }, h("span", { class: "ring" }), h("strong", {}, n.label), h("span", {}, n.detail)))));

  const activity = d.crashes.length
    ? card("grow",
        h("h2", { class: "card-title" }, "Recent crashes"),
        h("ul", { class: "rows" }, d.crashes.slice(0, 5).map((c) => h("li", {}, h("span", { class: "when" }, c.when), h("span", {}, c.what)))))
    : card("grow",
        h("h2", { class: "card-title" }, "Recent starts"),
        d.launches.length
          ? h("ul", { class: "rows" }, d.launches.slice(0, 4).map(launchRow))
          : h("p", { class: "muted" }, s.enabled ? "Start the game from Steam and it shows up here." : "Starts show up here once CachyPlay is on."));

  out.push(h("div", { class: "overview" }, left, h("div", { class: "stack" }, path, activity)));
  if (ins && ins.working && ins.working.length && ins.summary && ins.summary.light === "red") out.push(compareCard(ins));
  return out;
}

/* This game next to games that run fine on the same PC. */
function compareCard(ins) {
  const tool = (t) => (t ? t.replace(/-x86_64$/, "") : "Native Linux");
  const opts = (o) => (o && o.trim() ? o.trim() : "None");
  const rows = [ins.this_setup, ...ins.working.slice(0, 3)];
  return card("compare",
    h("h2", { class: "card-title" }, "Compared with games that run fine"),
    h("p", { class: "muted" }, "Same PC, same driver. Differences are worth trying one at a time."),
    h("table", { class: "compare" },
      h("thead", {}, h("tr", {}, h("th", {}, ""), ...rows.map((r, i) => h("th", { class: i === 0 ? "this" : "" }, r.name)))),
      h("tbody", {},
        h("tr", {}, h("th", {}, "Status"), ...rows.map((r, i) => h("td", { class: i === 0 ? "this bad" : "good" }, i === 0 ? "Not working" : `Ran ${duration(r.minutes * 60)}`))),
        h("tr", {}, h("th", {}, "Proton"), ...rows.map((r, i) => h("td", { class: i === 0 ? "this" : "" }, tool(r.tool)))),
        ins.this_graphics ? h("tr", {}, h("th", {}, "Graphics"), h("td", { class: "this" }, ins.this_graphics), ...rows.slice(1).map(() => h("td", { class: "muted" }, "—"))) : null,
        h("tr", {}, h("th", {}, "Launch options"), ...rows.map((r, i) => h("td", { class: i === 0 ? "this" : "" }, h("code", {}, opts(r.launch_options))))))));
}

const STAGE_CLASS = { not_working: "bad", testing: "warn", seems_to_work: "warn", works_with_fix: "good", runs_fine: "good", unknown: "muted" };

function progressCard(d, p) {
  const steps = [["testing", "Testing the fix"], ["seems_to_work", "Seems to work"], ["works_with_fix", "Works with a fix"]];
  const at = steps.findIndex((x) => x[0] === p.stage);
  return card("progress",
    h("h2", { class: "card-title " + (STAGE_CLASS[p.stage] || "") }, p.headline),
    p.detail ? h("p", {}, p.detail) : null,
    p.fix_label && at >= 0
      ? h("ol", { class: "steps", "aria-label": "Status steps" }, steps.map((x, i) => h("li", { class: i < at ? "done" : i === at ? "now" : "" }, x[1])))
      : null,
    h("p", { class: "muted" }, "A clean run is " + p.needed_text + (p.online_only ? ", online." : ".")),
    h("label", { class: "check" },
      h("input", { type: "checkbox", checked: p.online_only, onchange: (e) => setOnlineOnly(d.summary.app_id, e.target.checked) }),
      " The problem only happens online (only online starts count)"));
}

function eventRow(it) {
  return h("li", { class: "tl-event tl-" + it.what }, h("span", { class: "when" }, when(it.at)), h("span", {}, it.text));
}

const NOTE_CHOICES = [["fine", "Ran fine"], ["black_screen", "Black screen"], ["stutter", "Stutter"], ["glitches", "Graphics glitches"], ["crash", "Crashed to desktop"], ["network", "Network lag"]];

function runRow(d, r, items, ins) {
  const newestEnded = items.find((x) => x.kind === "run" && !x.running);
  const ask = !r.running && (r === newestEnded && !(r.note && r.note.kind) || (ins.progress && ins.progress.ask_online === r.start));
  const cls = { crash: "bad", problem: "bad", didnt_start: "bad", short: "warn", running: "warn", fine: "good" }[r.outcome] || "muted";
  const chips = [
    r.tool ? h("span", { class: "chip" }, r.tool.replace(/-x86_64$/, "")) : null,
    r.through_cachyplay
      ? (r.applied.length ? h("span", { class: "chip" }, "With: " + r.applied.map((a) => a.replace(/^Fix: /, "").split(":")[0]).join(", ")) : h("span", { class: "chip" }, "CachyPlay on"))
      : h("span", { class: "chip muted" }, "CachyPlay off"),
    r.gpu ? h("span", { class: "chip" + (r.gpu_integrated ? " warn" : "") }, "On " + r.gpu.replace(/^the /, "")) : null,
    r.counts ? h("span", { class: "chip " + (r.counts.startsWith("✓") ? "good" : "muted") }, r.counts) : null,
    r.note && r.note.kind ? h("span", { class: "chip" }, NOTE_CHOICES.find((c) => c[0] === r.note.kind)[1] + (r.note.online === true ? " · online" : r.note.online === false ? " · offline" : "")) : null,
  ];
  const isOpen = state.openRuns && state.openRuns.has(r.start);
  const details = h("div", { class: "tl-details", hidden: !isOpen },
    r.crash_detail ? h("p", {}, r.crash_detail) : null,
    h("p", { class: "muted" }, [
      r.secs != null ? "Ran " + duration(r.secs) : null,
      r.exit_code != null ? "exit code " + r.exit_code : null,
      r.applied.length ? "CachyPlay applied: " + r.applied.join("; ") : null,
    ].filter(Boolean).join(" · ")));
  return h("li", { class: "tl-run" },
    h("button", { class: "tl-head", "aria-expanded": String(!!isOpen), onclick: (e) => { const b = e.currentTarget; const open = b.getAttribute("aria-expanded") !== "true"; b.setAttribute("aria-expanded", String(open)); details.hidden = !open; if (open) state.openRuns.add(r.start); else state.openRuns.delete(r.start); } },
      h("span", { class: "when" }, when(r.start)),
      h("span", { class: cls }, r.text)),
    h("div", { class: "chips" }, chips),
    details,
    ask ? noteAsk(d, r, ins) : null);
}

function noteAsk(d, r, ins) {
  const onlineOnly = ins.progress && ins.progress.online_only;
  const kind = r.note && r.note.kind;
  return h("div", { class: "note-ask" },
    !kind ? h("span", { class: "muted" }, "What happened?") : null,
    !kind ? h("div", { class: "actions" }, NOTE_CHOICES.map(([k, label]) => h("button", { class: "secondary small", onclick: () => setRunNote(d.summary.app_id, r.start, k, null) }, label))) : null,
    (r.note && r.note.online != null) ? null : h("div", { class: "actions" },
      h("span", { class: "muted" }, onlineOnly ? "Was that online? (only online starts count)" : "Online or offline?"),
      h("button", { class: "secondary small", onclick: () => setRunNote(d.summary.app_id, r.start, "", true) }, "Online"),
      h("button", { class: "secondary small", onclick: () => setRunNote(d.summary.app_id, r.start, "", false) }, "Offline")));
}

async function setRunNote(appId, start, kind, online) {
  try {
    await invoke("set_run_note", { appId, start, kind, online });
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function setOnlineOnly(appId, on) {
  try {
    await invoke("set_online_only", { appId, on });
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

function launchRow(l) {
  return h("li", {},
    h("span", { class: "when" }, when(l.started_at)),
    h("span", {}, l.running ? "" : duration(l.runtime_secs)),
    l.running ? h("span", { class: "warn" }, "Running now, or stopped responding")
      : l.failed ? h("span", { class: "bad" }, "Closed early") : h("span", { class: "good" }, "Ran normally"),
    l.gpu ? h("span", { class: l.gpu_integrated ? "warn" : "muted" }, "On " + l.gpu.replace(/^the /, "")) : null);
}

/* ---------- suggested fixes ---------- */
const FIX_STATUS = {
  new: ["Suggested", ""],
  applied: ["Applied", "Start the game from Steam to try it."],
  worked: ["Worked", "The game has run normally since you applied this."],
  didnt_help: ["Ruled out", "The game crashed again with this. Try the next suggestion, or undo it."],
};

const CONFIDENCE = { likely: "Likely", possible: "Possible", unsure: "Long shot" };

function suggestionBody(appId, s) {
  const ins = insightsFor(appId);
  const out = [
    h("div", { class: "s-title" }, h("h2", {}, s.title), h("span", { class: `confidence ${s.confidence}`, title: "How sure CachyPlay is" }, CONFIDENCE[s.confidence])),
    h("ul", { class: "why" }, s.why.map((w) => h("li", {}, w))),
    s.proof ? h("p", { class: "proof" }, h("strong", {}, "What would prove it: "), s.proof) : null,
  ];
  // Only fixes CachyPlay applied have a "worked / didn't help" story to tell.
  if (s.status !== "new" && (s.fix || s.switched)) out.push(h("p", { class: `s-note ${s.status}` }, FIX_STATUS[s.status][1]));
  if (s.steps.length) out.push(h("ol", { class: "steps" }, s.steps.map((st) => h("li", {}, st))));
  if (s.switch_tool) {
    const blocked = editBlocked();
    out.push(h("div", { class: "actions" },
      s.switched
        ? h("button", { class: "secondary", disabled: !!blocked && !canQueueNow(), title: blocked || "", onclick: () => compatAction("undo_compat_tool", { appId }) }, canQueueNow() ? "Undo when Steam closes" : "Undo")
        : h("button", { class: "primary", disabled: !!blocked && !canQueueNow(), title: blocked || "", onclick: () => compatAction("switch_compat_tool", { appId, name: s.switch_tool.name }) }, canQueueNow() ? `Switch to ${s.switch_tool.display} when Steam closes` : `Switch to ${s.switch_tool.display}`),
      blocked && !canQueueNow() ? h("span", { class: "muted" }, "Close Steam first.") : null));
  }
  if (s.action === "reset_shader_cache") {
    const cacheItem = ins && ins.applied.find((a) => a.id === "shader_cache");
    out.push(h("div", { class: "actions" }, cacheItem
      ? h("button", { class: "secondary", onclick: () => undoChange(appId, cacheItem) }, "Undo (put the old cache back)")
      : h("button", { class: "primary", onclick: async () => (await okToStack(appId)) && confirmDialog("Start the shader cache fresh?", "The current cache is moved aside, not deleted. Close the game first. The first start afterwards stutters a little while shaders rebuild.", "Reset cache").then((ok) => ok && simpleAction("reset_shader_cache", appId)) }, "Reset shader cache")));
  }
  if (s.fix) {
    const tech = [...Object.entries(s.fix.env).map(([k, v]) => `${k}=${v}`), ...s.fix.args].join(" ");
    out.push(h("details", { class: "tech-details" }, h("summary", {}, "Show the technical setting"), h("code", { class: "tech" }, tech)));
    const item = ins && ins.applied.find((a) => a.id === s.fix.id);
    out.push(h("div", { class: "actions" }, item
      ? h("button", { class: "secondary", onclick: () => undoChange(appId, item) }, "Undo")
      : s.try_again
        ? h("button", { class: "secondary", onclick: (e) => tryAgain(appId, s.id).then(() => applyFix(appId, s.id, e.currentTarget)) }, "Try again")
        : h("button", { class: "primary", onclick: (e) => applyFix(appId, s.id, e.currentTarget) }, "Apply fix"),
      s.status === "worked" && ins && ins.share_urls[s.id]
        ? h("button", { class: "secondary", title: "Opens a GitHub page you can read before sending. It helps make this a tested fix for everyone.", onclick: () => openUrl(ins.share_urls[s.id]) }, "Share that this worked")
        : null));
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
  const ins = insightsFor(d.summary.app_id);
  return [
    appliedCard(d),
    ins && ins.find_cause && ins.find_cause.steps.length ? findCauseCard(ins.find_cause) : null,
    h("p", { class: "muted intro" }, "Worked out on your PC from crashes, logs and how your other games run, most likely first. Every fix can be undone. Labels: Likely (two or more clues agree), Possible (one clue), Long shot (a general fix)."),
    h("ol", { class: "fix-list" }, list.map((s, i) => h("li", {},
      card((i === 0 && s.status === "new" ? "recommended " : "") + (s.big_step ? "big-step" : ""),
        h("div", { class: "fix-head" },
          h("span", { class: "num" }, String(i + 1)),
          s.big_step ? h("span", { class: "pill big" }, "Big step") : null,
          s.status !== "new" ? h("span", { class: `pill ${s.status}` }, FIX_STATUS[s.status][0]) : null),
        ...suggestionBody(d.summary.app_id, s))))),
  ];
}

const STEP_LABEL = { ruled_out: "Ruled out", helped: "Helped", testing: "Testing", next: "Next", todo: "" };

function findCauseCard(fc) {
  return card("find-cause",
    h("h2", { class: "card-title" }, "Find the cause"),
    h("p", {}, fc.summary),
    h("ol", { class: "ladder" }, fc.steps.map((st) => h("li", { class: "ladder-" + st.status },
      h("span", { class: "ladder-tag" }, STEP_LABEL[st.status]),
      h("span", {}, st.title)))),
    h("p", { class: "muted" }, "One change per start. Each result is recorded, and anything that crashes anyway is crossed off, including things you try in Steam's launch options."));
}

async function simpleAction(cmd, appId) {
  try {
    toast(await invoke(cmd, { appId }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

function addonsCard(d) {
  const ins = insightsFor(d.summary.app_id);
  const extra = ((ins && ins.layers) || []).filter((l) => !l.trusted);
  if (!extra.length) return null;
  return card("addons",
    h("h2", { class: "card-title" }, "Vulkan add-ons on this PC"),
    h("p", { class: "muted" }, "Extra programs that can load into games. Only active ones affect games; \"Vulkan add-ons off\" is only offered when one is active."),
    h("ul", { class: "rows" }, extra.map((l) => h("li", {},
      h("span", {}, h("strong", {}, l.name), " ", h("span", { class: "muted" }, l.what)),
      h("span", { class: l.active ? "warn" : "muted" }, l.active ? "Active: loads into every game" : "Installed, not active")))));
}

function yourSettingsCard(d) {
  const s = d.summary;
  const input = h("input", { type: "text", class: "settings-input", value: d.your_settings || "", placeholder: "VKD3D_CONFIG=no_upload_hvv -force-d3d12", "aria-label": `Settings to try for ${s.name}`, spellcheck: "false" });
  const save = async () => {
    if (input.value.trim() && input.value.trim() !== (d.your_settings || "") && !(await okToStack(s.app_id))) return;
    try {
      toast(await invoke("set_your_settings", { appId: s.app_id, text: input.value }), "success");
    } catch (e) {
      toast(String(e), "error");
    }
    state.insights.delete(s.app_id);
    input.blur();
    await load();
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") save(); });
  return card("your-settings",
    h("h2", { class: "card-title" }, "Try these settings"),
    h("p", { class: "muted" }, "Settings (NAME=value) and game options (like -force-d3d12) to test, without touching Steam's launch options. They work with Steam open, and CachyPlay counts each try toward the status steps. Leave out %command%."),
    h("div", { class: "settings-row" }, input,
      h("button", { class: "primary", onclick: save }, "Save"),
      d.your_settings ? h("button", { class: "secondary", onclick: () => { input.value = ""; save(); } }, "Remove") : null),
    !s.enabled ? h("p", { class: "s-note warn" }, "CachyPlay is off for this game, so these aren't used until it's on.") : null);
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
  const out = [yourSettingsCard(d), appliedCard(d), addonsCard(d), card("",
    h("h2", { class: "card-title" }, s.enabled ? "What CachyPlay changes" : "What CachyPlay would change"),
    h("table", { class: "changes" },
      h("thead", {}, h("tr", {}, h("th", {}, "Setting"), h("th", {}, "Stock"), h("th", {}, "CachyPlay"))),
      h("tbody", {}, rows)),
    d.fix && !d.fix.tested ? h("p", { class: "note untested" }, "This game's fix file hasn't been tested on real hardware yet. It only uses general, low-risk settings.") : null,
    !d.fix ? h("p", { class: "note" }, "No fix file for this game yet. CachyPlay still records each launch and helps with bug reports.") : null,
    d.fix && d.fix.notes ? h("p", { class: "note" }, d.fix.notes) : null)];

  if (d.graphics_card) out.push(cardCard(d));
  out.push(obsCard(d));

  if (d.standalone_launch_options) {
    out.push(card("keep",
      h("h2", { class: "card-title" }, "Keep these settings without CachyPlay"),
      h("p", { class: "muted" }, "Paste this into the game's Launch options in Steam. Uninstalling with \"Keep the settings\" does this for you."),
      d.summary.enabled ? h("p", { class: "s-note warn" }, "Pasting this replaces CachyPlay's part of the launch options, so CachyPlay turns off for this game. To test settings and keep CachyPlay on, use \"Try these settings\" above.") : null,
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
  out.push(prefixCard(d));
  out.push(h("details", { class: "raw" }, h("summary", {}, "Show Steam's launch options for this game"), h("code", {}, d.launch_options || "(empty)")));
  return out;
}

/* Reset the game's Windows files (Proton prefix), with Undo. */
function prefixCard(d) {
  const ins = insightsFor(d.summary.app_id);
  if (!ins || (!ins.has_prefix && !ins.kept_prefixes)) return null;
  const blocked = editBlocked();
  return card("prefix",
    h("h2", { class: "card-title" }, "Reset the game's Windows files"),
    h("p", { class: "muted" }, "Proton keeps a small Windows setup for each game. If it gets damaged, the game can crash at start no matter what else you change. Resetting sets it aside (nothing is deleted) and Steam builds a fresh one next time. Saves kept there come back with Undo; most games also keep saves in Steam Cloud."),
    h("div", { class: "actions" },
      ins.has_prefix ? h("button", { class: "secondary", disabled: !!blocked && !canQueueNow(), title: blocked || "", onclick: () => prefixAction("reset_prefix", d.summary.app_id, d.summary.name) }, canQueueNow() ? "Reset when Steam closes" : "Reset Windows files") : null,
      ins.kept_prefixes ? h("button", { class: "secondary", disabled: !!blocked, title: blocked || "", onclick: () => prefixAction("undo_prefix_reset", d.summary.app_id, d.summary.name) }, "Undo reset") : null,
      ins.kept_prefixes ? h("button", { class: "secondary", disabled: !!blocked, title: blocked || "", onclick: () => prefixAction("delete_kept_prefixes", d.summary.app_id, d.summary.name, ins.kept_size) }, ins.kept_size ? `Delete old copies (${ins.kept_size})` : "Delete old copies") : null,
      blocked ? h("span", { class: "muted" }, "Close Steam first.") : null));
}

function cardCard(d) {
  const c = d.graphics_card;
  let note;
  if (!c.on) note = `Off. The game picks a chip by itself, which can be ${c.builtin}.`;
  else if (!d.summary.enabled) note = "On, but CachyPlay is off for this game, so it isn't used. Turn CachyPlay on above.";
  else if (c.last_was_builtin) note = `The last start still ran on ${c.builtin}. Check the game's own graphics settings for a display adapter option.`;
  else if (c.last_used) note = `On. The last start ran on ${c.last_used}.`;
  else note = `On. CachyPlay checks which chip the game uses the next time it runs.`;
  return card("gpu",
    h("div", { class: "obs-row" },
      h("div", {},
        h("h2", { class: "card-title" }, "Use the graphics card"),
        h("p", { class: "muted" }, `This PC has ${c.card} and ${c.builtin}. Games should draw on the card; the built-in chip is much slower.`)),
      h("button", {
        class: "switch",
        role: "switch",
        "aria-checked": c.on ? "true" : "false",
        "aria-label": `Use the graphics card for ${d.summary.name}`,
        disabled: state.busy,
        onclick: () => setPreferCard(d.summary.app_id, !c.on),
      })),
    h("p", { class: c.last_was_builtin && c.on ? "s-note warn" : "s-note" }, note));
}

async function setPreferCard(appId, on) {
  state.busy = true;
  try {
    toast(await invoke("set_prefer_card", { appId, on }), "success");
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    await load();
  }
}

function obsCard(d) {
  const o = d.obs;
  const blocked = editBlocked();
  // Turning it on may need to turn CachyPlay on too, which needs Steam closed and CachyPlay installed.
  const needsEnable = !d.summary.enabled;
  const disabled = state.busy || o.in_your_options || (needsEnable && !o.on && (!!blocked || !state.overview.installed_cli));
  let note;
  if (o.in_your_options) note = "Already on: your own Steam launch options start this game with obs-gamecapture.";
  else if (o.on && needsEnable) note = "On, but CachyPlay is off for this game, so it isn't used. Turn CachyPlay on above.";
  else if (o.all && o.on) note = "On for every game (change it in Settings).";
  else if (o.on) note = "On. In OBS, add a Game Capture source, then start the game from Steam.";
  else if (needsEnable && blocked) note = `${blocked} Turning this on also turns CachyPlay on for the game.`;
  else if (needsEnable) note = "Turning this on also turns CachyPlay on for this game.";
  else note = "Off. OBS records your screen as usual.";
  return card("obs",
    h("div", { class: "obs-row" },
      h("div", {},
        h("h2", { class: "card-title" }, "OBS game capture"),
        h("p", { class: "muted" }, "Records just this game with OBS's Game Capture source, instead of your whole desktop. Same as ", h("code", {}, "obs-gamecapture %command%"), " in Steam.")),
      h("button", {
        class: "switch",
        role: "switch",
        "aria-checked": o.on || o.in_your_options ? "true" : "false",
        "aria-label": `OBS game capture for ${d.summary.name}`,
        disabled,
        title: disabled && blocked ? blocked : "",
        onclick: () => setObs(d.summary.app_id, !o.on),
      })),
    h("p", { class: "s-note" }, note),
    !o.installed && !o.in_your_options
      ? h("div", { class: "note obs-install" },
          h("p", {}, "obs-gamecapture isn't installed yet. Install it once in a terminal:"),
          h("code", { class: "kept-line" }, o.install_command),
          h("div", { class: "actions" }, h("button", { class: "secondary", onclick: () => copyText(o.install_command, "Copied. Paste it into a terminal.") }, "Copy")))
      : null);
}

function historyTab(d) {
  const ins = insightsFor(d.summary.app_id);
  const out = [];
  if (ins && ins.progress) out.push(progressCard(d, ins.progress));
  const items = (ins && ins.timeline) || [];
  out.push(card("starts",
    h("h2", { class: "card-title" }, "Recent starts"),
    items.length
      ? h("ol", { class: "timeline" }, items.map((it, i) => it.kind === "run" ? runRow(d, it, items, ins) : eventRow(it)))
      : h("p", { class: "muted" }, "No starts in Steam's log yet. Start the game from Steam and it shows up here.")));
  if (!items.some((it) => it.kind === "run" && it.outcome === "crash") && d.crashes.length) {
    out.push(card("",
      h("h2", { class: "card-title" }, "Graphics driver crashes"),
      h("ul", { class: "rows" }, d.crashes.map((c) => h("li", {}, h("span", { class: "when" }, c.when), h("span", {}, c.what))))));
  }
  if (ins && ins.nvidia_report) {
    out.push(card("nvidia",
      h("h2", { class: "card-title" }, "Tell NVIDIA"),
      h("p", { class: "muted" }, "The same driver error keeps happening. Run sudo nvidia-bug-report.sh in a terminal, then post this on NVIDIA's Linux forum with nvidia-bug-report.log.gz and a CachyPlay bug report attached."),
      h("pre", { class: "dev-text" }, ins.nvidia_report),
      h("div", { class: "actions" },
        h("button", { class: "primary", onclick: () => copyText(ins.nvidia_report, "Copied. Paste it into your post.") }, "Copy text"),
        h("button", { class: "secondary", onclick: () => openUrl("https://forums.developer.nvidia.com/c/gpu-graphics/linux/148") }, "Open NVIDIA's Linux forum"))));
  }
  if (ins && ins.developer_report) {
    out.push(card("dev",
      h("h2", { class: "card-title" }, "Tell the developer"),
      h("p", { class: "muted" }, "Developers fix what they can see. Copy this, post it in the game's Steam discussions (or the developer's Discord), and attach a bug report."),
      h("pre", { class: "dev-text" }, ins.developer_report),
      h("div", { class: "actions" },
        h("button", { class: "primary", onclick: () => copyText(ins.developer_report, "Copied. Paste it into your post.") }, "Copy text"),
        h("button", { class: "secondary", onclick: () => openUrl(ins.discussions_url) }, "Open Steam discussions"),
        h("button", { class: "secondary", onclick: () => saveReport(d.summary.app_id) }, "Save bug report"))));
  }
  return out;
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
/// Changing two things before the next start hides which one helped: ask first.
async function okToStack(appId) {
  const changed = insightsFor(appId)?.changed_since_start || [];
  if (!changed.length) return true;
  return confirmDialog("Change another thing before testing?",
    `You already changed ${changed.join(", ")} since the game last started. Changing two things at once means you won't know which one helped. Start the game once first to test it, or apply anyway.`,
    "Apply anyway");
}

async function applyFix(appId, id, btn) {
  if (!(await okToStack(appId))) return;
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Applying…";
  }
  try {
    toast(await invoke("apply_fix", { appId, id }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function compatAction(cmd, args) {
  if (cmd === "switch_compat_tool" && !(await okToStack(args.appId))) return;
  try {
    if (canQueueNow()) {
      const action = cmd === "undo_compat_tool" ? { action: "undo_tool", app_id: args.appId } : { action: "switch_tool", app_id: args.appId, tool: args.name };
      toast(await invoke("queue_action", { action }), "success", { label: "Close Steam for me", run: closeSteam });
    } else {
      toast(await invoke(cmd, args), "success");
    }
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(args.appId);
  await load();
}

async function undoFix(appId, id) {
  try {
    toast(await invoke("undo_fix", { appId, id }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function fixIt(appId, btn) {
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Fixing…";
  }
  try {
    toast(await invoke("fix_it", { appId }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function setVerdict(appId, id, worked) {
  try {
    toast(await invoke("set_verdict", { appId, id, worked }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function prefixAction(cmd, appId, name, size) {
  if (cmd === "delete_kept_prefixes") {
    const ok = await confirmDialog(`Delete the old copies of ${name}'s Windows files?`,
      `This frees ${size || "their space"}, but Undo reset won't be possible any more. Saves kept only in those copies are deleted too. The game's current files aren't touched.`,
      "Delete");
    if (!ok) return;
  }
  if (cmd === "reset_prefix") {
    const ok = await confirmDialog(`Reset ${name}'s Windows files?`,
      "They're set aside, not deleted, and Undo puts them back. The game's first start afterwards takes a little longer while Steam sets things up again.",
      "Reset");
    if (!ok) return;
  }
  try {
    if (cmd === "reset_prefix" && canQueueNow()) {
      toast(await invoke("queue_action", { action: { action: "reset_prefix", app_id: appId } }), "success", { label: "Close Steam for me", run: closeSteam });
    } else {
      toast(await invoke(cmd, { appId }), "success");
    }
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function openUrl(url) {
  try {
    await invoke("open_url", { url });
  } catch (e) {
    toast(String(e), "error");
  }
}

function canQueueNow() {
  const st = state.overview && state.overview.steam;
  return !!(st && st.can_queue && st.blocked_reason);
}

async function setEnabled(appId, on) {
  state.busy = true;
  try {
    if (canQueueNow()) {
      toast(await invoke("queue_action", { action: { action: on ? "enable" : "disable", app_id: appId } }), "success", { label: "Close Steam for me", run: closeSteam });
      return;
    }
    toast(await invoke("set_enabled", { appId, enabled: on }), "success");
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    await load();
  }
}

async function cancelWaiting(appId) {
  try {
    await invoke("cancel_waiting", { appId });
  } catch (e) {
    toast(String(e), "error");
  }
  await load();
}

async function closeSteam() {
  try {
    toast(await invoke("close_steam"));
  } catch (e) {
    toast(String(e), "error");
  }
}

async function openSteam() {
  try {
    toast(await invoke("open_steam"));
  } catch (e) {
    toast(String(e), "error");
  }
}

/* ---------- staying up to date ---------- */
state.live = null;
state.lastLoad = Date.now();
state.openRuns = new Set();
async function poll() {
  let l;
  try {
    l = await invoke("live_status");
  } catch {
    return;
  }
  const prev = state.live;
  state.live = l;
  if (!prev) return;
  // Don't redraw under someone who's typing or has a dialog open.
  const typing = document.activeElement && ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName);
  const dialog = document.querySelector("dialog[open]");
  if (prev.steam !== "closed" && l.steam === "closed" && l.waiting.length) {
    try {
      const lines = await invoke("run_waiting");
      lines.forEach((x) => toast(x, x.startsWith("Done") ? "success" : "error"));
      toast("Steam is closed and your changes are in.", "success", { label: "Open Steam", run: openSteam });
    } catch (e) {
      toast(String(e), "error");
    }
  }
  const changed = prev.steam !== l.steam || prev.token !== l.token;
  if (changed && !typing && !dialog && !state.busy) {
    // Keep the loaded suggestions on screen while fresh ones load (no empty placeholders).
    state.lastLoad = Date.now();
    await load();
  } else if (prev.steam !== l.steam) {
    renderChrome();
  } else if (!typing && !dialog && Date.now() - state.lastLoad > 60000) {
    // While a game runs, only the "Playing now, 12 of 18 min" card ticks over.
    const ins = state.insights.get(state.selected);
    if (ins?.progress?.playing_secs != null) {
      state.lastLoad = Date.now();
      try {
        const fresh = await invoke("game_insights", { appId: state.selected });
        state.insights.set(state.selected, fresh);
        const old = document.querySelector(".card.progress");
        if (old && state.detail) old.replaceWith(progressCard(state.detail, fresh.progress));
      } catch {
        /* next tick */
      }
    }
  }
}
setInterval(poll, 3000);

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
    state.update = await invoke("update_status", { force: !silent });
  } catch (e) {
    state.update = { error: String(e) };
  }
  renderUpdate();
}

function renderUpdate() {
  const u = state.update;
  const pill = $("update-pill");
  const line = $("update-line");
  const btn = $("update-now-settings");
  if (!u) return;
  for (const r of document.querySelectorAll('input[name="channel"]')) r.checked = r.value === u.channel;
  $("roll-back").hidden = !u.can_roll_back;
  btn.hidden = true;
  if (u.error) {
    pill.hidden = true;
    line.className = "status-line bad";
    line.textContent = u.error;
    return;
  }
  if (u.available && u.latest) {
    pill.hidden = false;
    pill.textContent = `Update to ${u.latest.tag}`;
    line.className = "status-line new";
    line.textContent = `${u.latest.tag} is available. You have v${u.current}.`;
    btn.hidden = false;
    btn.textContent = `Update to ${u.latest.tag}`;
    btn.onclick = openUpdateDialog;
  } else if (u.newer_beta) {
    pill.hidden = true;
    line.className = "status-line new";
    line.textContent = `You have the newest Stable version (v${u.current}). A newer beta, ${u.newer_beta.tag}, is out.`;
    btn.hidden = false;
    btn.textContent = `Switch to Beta and update`;
    btn.onclick = async () => {
      await setChannel("beta");
      openUpdateDialog();
    };
  } else if (u.stable_now && u.kind === "user") {
    pill.hidden = true;
    line.className = "status-line new";
    line.textContent = `You're on beta v${u.current}, which is newer than the newest Stable version (${u.stable_now.tag}). You'll get Stable updates from the next one on, or switch now.`;
    btn.hidden = false;
    btn.textContent = `Switch to Stable now (${u.stable_now.tag})`;
    btn.onclick = switchToStableNow;
  } else if (u.no_releases) {
    pill.hidden = true;
    line.className = "status-line";
    line.textContent = `You have v${u.current}. No CachyPlay release has been published on GitHub yet, so there's nothing to update to.`;
  } else {
    pill.hidden = true;
    line.className = "status-line good";
    line.textContent = `You have the newest ${CHANNEL_NAME[u.channel]} version (v${u.current}).`;
  }
}

async function switchToStableNow() {
  const btn = $("update-now-settings");
  btn.disabled = true;
  btn.textContent = "Installing…";
  try {
    const msg = await invoke("install_update", { toStable: true });
    toast(msg, "success");
    await invoke("restart_app");
  } catch (e) {
    toast(String(e), "error");
  } finally {
    btn.disabled = false;
    renderUpdate();
  }
}

function openUpdateDialog() {
  const u = state.update;
  if (!u || !u.latest || !u.available) return;
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
    $("update-sub").textContent = `You have v${u.current}, running from a download or build folder. Install CachyPlay first, then update.`;
    btn.textContent = "Install CachyPlay";
    btn.onclick = async () => {
      $("update-dialog").close();
      await installSelf();
    };
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
  await checkUpdates(false);
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

function chooseDialog(title, text, choices) {
  return new Promise((resolve) => {
    const dlg = $("choose");
    $("choose-title").textContent = title;
    $("choose-text").textContent = text;
    $("choose-actions").replaceChildren(
      ...choices.map(([value, label, cls]) => h("button", { class: cls || "secondary", value }, label)),
      h("button", { class: "secondary", value: "cancel" }, "Cancel"));
    dlg.onclose = () => resolve(dlg.returnValue && dlg.returnValue !== "cancel" ? dlg.returnValue : null);
    dlg.returnValue = "";
    dlg.showModal();
    // Keyboard starts on Cancel, so Enter never crosses something off by accident.
    $("choose-actions").lastElementChild.focus();
  });
}

const UNDO_REASONS = [["worse", "It made things worse", "danger-btn"], ["didnt_help", "It didn't help"], ["testing", "Just testing"]];

async function undoChange(appId, item) {
  let reason = "testing";
  if (!item.temporary) {
    reason = await chooseDialog(`Undo "${item.label}"?`, "Why? \"Made things worse\" and \"Didn't help\" cross it off for this game so it isn't suggested again. \"Just testing\" doesn't.", UNDO_REASONS);
    if (!reason) return;
  }
  try {
    toast(await invoke("undo_change", { appId, id: item.id, reason }), "success");
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

async function undoAll(appId, items) {
  const reason = await chooseDialog("Undo everything CachyPlay applies to this game?", `${items.length} change${items.length === 1 ? "" : "s"}: ${items.map((i) => i.label).join(", ")}. Why?`, UNDO_REASONS);
  if (!reason) return;
  for (const item of items) {
    try {
      await invoke("undo_change", { appId, id: item.id, reason: item.temporary ? "testing" : reason });
    } catch (e) {
      toast(String(e), "error");
    }
  }
  toast("Everything is undone. The game starts plain next time.", "success");
  state.insights.delete(appId);
  await load();
}

async function tryAgain(appId, id) {
  try {
    await invoke("try_again", { appId, id });
  } catch (e) {
    toast(String(e), "error");
  }
  state.insights.delete(appId);
  await load();
}

function appliedCard(d) {
  const ins = insightsFor(d.summary.app_id);
  const items = (ins && ins.applied) || [];
  if (!items.length) return null;
  const KIND = { fix: "Fix", settings: "Your settings", proton: "Proton", shader_cache: "Shader cache" };
  return card("applied",
    h("h2", { class: "card-title" }, "Applied for this game"),
    h("p", { class: "muted" }, d.summary.enabled ? "Everything CachyPlay adds when this game starts. Steam's launch options don't show these." : "CachyPlay is off for this game, so these aren't used right now (except a Proton switch, which is set in Steam)."),
    h("ul", { class: "rows applied-list" }, items.map((it) => h("li", { class: it.worse ? "worse" : "" },
      h("span", {},
        h("span", { class: "chip" }, KIND[it.kind] || "Fix"), " ", h("strong", {}, it.label.replace(/^Your settings: /, "")),
        h("span", { class: "muted" }, it.since ? ` · since ${it.since}` : ""),
        it.temporary ? h("span", { class: "muted" }, " · for one start") : null,
        it.worse ? h("p", { class: "s-note warn" }, "This made things worse for this game: " + it.worse + " Undo it.") : null),
      h("button", { class: it.worse ? "primary" : "secondary", onclick: () => undoChange(d.summary.app_id, it) }, "Undo")))),
    items.length > 1 ? h("div", { class: "actions" }, h("button", { class: "link", onclick: () => undoAll(d.summary.app_id, items) }, "Undo all")) : null);
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
  renderObsSettings();
  renderAppSettings();
  if (!$("pc-checks").children.length) runPcCheck();
  $("games-view").hidden = true;
  $("settings").hidden = false;
  $("settings-content").scrollTop = 0;
  $("close-settings").focus();
}

function closeSettings() {
  $("settings").hidden = true;
  $("games-view").hidden = false;
}

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
  if (!$("settings").hidden) return;
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
}

async function setAppSetting(key, value) {
  try {
    await invoke("set_app_setting", { key, value });
    state.settings[key] = value;
  } catch (e) {
    toast(String(e), "error");
  }
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
  let accent = null;
  if (theme === "system") {
    try {
      accent = await invoke("desktop_accent");
    } catch {
      accent = null;
    }
  }
  if (accent) document.documentElement.style.setProperty("--accent", accent);
  else document.documentElement.style.removeProperty("--accent");
  if (st.big_screen) startGamepad();
}

/* Big-screen mode: a controller moves around like the arrow keys. A = press, B = back. */
let gamepadTimer = null;
const padState = {};
function startGamepad() {
  if (gamepadTimer || !navigator.getGamepads) return;
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
    else for (const d of document.querySelectorAll("dialog[open]")) d.close();
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
    closeSettings();
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
$("obs-all").addEventListener("change", (e) => setObsAll(e.target.checked));
for (const r of document.querySelectorAll('input[name="channel"]')) {
  r.addEventListener("change", () => r.checked && setChannel(r.value));
}
$("restore-all").addEventListener("click", restoreAll);
$("uninstall").addEventListener("click", uninstall);
$("rail").addEventListener("keydown", railKeys);
$("close-settings").addEventListener("click", closeSettings);
$("run-pc-check").addEventListener("click", runPcCheck);
$("run-self-test").addEventListener("click", runSelfTest);
for (const r of document.querySelectorAll('input[name="theme"]')) {
  r.addEventListener("change", () => r.checked && setAppSetting("theme", r.value));
}
$("big-screen").addEventListener("change", (e) => setAppSetting("big_screen", e.target.checked));
$("protondb").addEventListener("change", (e) => setAppSetting("protondb", e.target.checked));
for (const a of document.querySelectorAll(".settings-nav a")) {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    document.querySelector(a.getAttribute("href")).scrollIntoView({ behavior: "smooth", block: "start" });
  });
}
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("settings").hidden && !document.querySelector("dialog[open]")) closeSettings();
});
window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyDisplay);
$("search").addEventListener("input", (e) => { state.query = e.target.value.trim().toLowerCase(); renderRail(); });

// Steam may have been opened or closed while CachyPlay was in the background.
// Suggestions are recomputed too, since a game may have crashed meanwhile.
window.addEventListener("focus", () => { state.insights.clear(); load(); });

loadAppSettings().then(load);
// Check for a newer CachyPlay once at startup (at most once a day online), in the background.
checkUpdates(true);
// Quietly check the PC; problems show as a banner.
runPcCheck();
