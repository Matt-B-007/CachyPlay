// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// A game's page: the header with the CachyPlay switch, the progress track, the mission brief
// (what happened / why / what to do, one main button), recent starts, and the tabs.

/* ---------- main area ---------- */
function renderEmptyMain() {
  if (state.overview && state.overview.games.length) {
    renderHome();
    return;
  }
  $("main").replaceChildren(...banners(),
    h("div", { class: "main-empty" }, h("img", { src: "logo.svg", alt: "", width: "72", height: "72" }),
      h("p", {}, state.overview.steam ? "Install a game in Steam and it appears here." : "Your games appear here once Steam is found.")));
}

async function showGame(appId, opts = {}) {
  state.selected = appId;
  markRail();
  const main = $("main");
  const scroll = main.scrollTop;
  try {
    state.detail = await invoke("game_detail", { appId });
  } catch (e) {
    main.replaceChildren(...banners(), h("div", { class: "main-empty" }, h("p", {}, String(e))));
    return;
  }
  if (state.selected !== appId) return;
  renderMain();
  if (opts.keepScroll) main.scrollTop = scroll;
  else main.scrollTop = 0;
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
  if (!d || !state.selected) {
    renderEmptyMain();
    return;
  }
  const main = $("main");
  const scroll = main.scrollTop;
  main.replaceChildren(...[...banners(), gameHead(d), track(d), tabs(d), h("div", { class: "tab-panel", role: "tabpanel", id: "tab-panel" }, ...panel(d))].filter(Boolean));
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

/* The status words: the game list's own words when they name the fix ("Works with a fix:
   Direct3D 12"), otherwise the stage. */
function stageLabel(d) {
  const p = insightsFor(d.summary.app_id)?.progress;
  if (!p) return null;
  if (p.stage === "testing" && p.playing_secs != null) return "Playing now";
  const listed = (state.overview.games.find((g) => g.app_id === d.summary.app_id) || d.summary).stage;
  if (p.stage === "works_with_fix" && listed && listed.startsWith("Works with a fix")) return listed;
  return STAGE_LABEL[p.stage] || null;
}

const LIGHT_LABEL = { green: "Runs fine", yellow: "Trying a fix", red: "Not working", grey: "Not played lately" };

/* The status chip's colour always matches its words: "Not working" is red, testing is amber,
   working (with or without a fix) is green. */
const STAGE_LIGHT = { not_working: "red", testing: "yellow", seems_to_work: "yellow", works_with_fix: "green", runs_fine: "green" };
function stageLight(d, light) {
  const p = insightsFor(d.summary.app_id)?.progress;
  return (p && STAGE_LIGHT[p.stage]) || light;
}

/* Starts in the timeline, newest first. */
function runsOf(ins) {
  return ((ins && ins.timeline) || []).filter((it) => it.kind === "run");
}

/* "Last start Nov 25 · proton-cachyos-slr · 7 crashes in 9 starts" */
function headFacts(d, ins) {
  const runs = runsOf(ins);
  const out = [];
  const p = ins.progress;
  if (runs.length) {
    out.push(`Last start ${shortDay(runs[0].start)}`);
    if (runs[0].tool) out.push(runs[0].tool.replace(/-x86_64$/, ""));
    const bad = runs.filter((r) => r.outcome === "crash" || r.outcome === "problem").length;
    // Once it works, old crashes aren't the news: say how it has run since.
    if (p && p.stage === "works_with_fix") out.push(`${p.clean_runs} clean ${p.clean_runs === 1 ? "run" : "runs"} with the fix`);
    else if (bad) out.push(`${bad} ${bad === 1 ? "crash" : "crashes"} in ${runs.length} ${runs.length === 1 ? "start" : "starts"}`);
    else out.push(`${runs.length} ${runs.length === 1 ? "start" : "starts"}, no crashes`);
  } else if (d.summary.enabled) {
    out.push("No starts yet");
  }
  return out.join(" · ");
}

function shortDay(iso) {
  const t = new Date(iso);
  if (isNaN(t)) return iso;
  const today = new Date();
  const y = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (t.toDateString() === today.toDateString()) return "today";
  if (t.toDateString() === y.toDateString()) return "yesterday";
  return t.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/* The top of a game: cover, name, status chip, a line of facts, and the CachyPlay switch (the one
   way to turn CachyPlay on or off for a game). */
function gameHead(d) {
  const s = d.summary;
  const ins = insightsFor(s.app_id);
  const sum = ins && ins.summary;
  const light = sum ? sum.light : (s.light || "grey");
  const rating = state.protondb.get(s.app_id);
  const listed = state.overview.games.find((g) => g.app_id === s.app_id) || s;
  const label = stageLabel(d);
  // Without a stage, the chip says what the game list says, in the same colour.
  const chipLight = label ? stageLight(d, light) : listed.light;
  return h("section", { class: "hero game-head" },
    h("div", { class: "hero-cover" }, h("span", {}, s.name.slice(0, 2).toUpperCase())),
    h("div", { class: "hero-text" },
      h("div", { class: "hero-title" },
        h("h1", {}, s.name),
        h("span", { class: `light-chip ${chipLight}` }, h("span", { class: `light ${chipLight}` }), label || statusText(listed)),
        rating ? h("button", { class: `protondb ${rating.tier}`, title: `${rating.meaning} (${rating.reports} reports on ProtonDB)`, onclick: () => openUrl(rating.url) }, `ProtonDB: ${rating.tier}`) : null),
      ins ? h("p", { class: "head-facts" }, headFacts(d, ins)) : null),
    switchCard(d));
}

function switchCard(d) {
  const s = d.summary;
  const blocked = editBlocked();
  const notInstalled = !state.overview.installed_cli;
  const canQueue = !!(blocked && state.overview.steam && state.overview.steam.can_queue);
  const canToggle = (!blocked || canQueue) && !(notInstalled && !s.enabled);
  const fixInSteam = d.fix_home && d.fix_home.in_steam && !s.enabled;
  const otherWaiting = !!s.waiting && !s.waiting.startsWith("Turn CachyPlay");
  const note = otherWaiting ? "A change waits for Steam to close"
    : s.waiting ? s.waiting.replace(/ for .*$/, "") + " when Steam closes"
    : canQueue ? (s.enabled ? "On · changes wait for Steam to close" : "Off · turning on waits for Steam to close")
    : blocked ? "Close Steam to change"
    : notInstalled && !s.enabled ? "Install CachyPlay first"
    : s.removed_in_steam ? "Off: Steam's launch options were edited"
    : fixInSteam ? "Off: the fix is in Steam"
    : s.enabled ? (d.fix_home && d.fix_home.by_cachyplay ? "On: adds this game's fix" : "On for this game")
    : "Off for this game";
  return h("div", { class: "switch-card" },
    h("span", { class: "switch-label" }, h("strong", {}, "CachyPlay"), h("span", {}, note)),
    h("button", {
      class: "switch",
      role: "switch",
      "aria-checked": s.enabled ? "true" : "false",
      "aria-label": `CachyPlay for ${s.name}`,
      // While another change waits (like keeping a fix in Steam), cancel it first.
      disabled: !canToggle || state.busy || otherWaiting,
      title: otherWaiting ? `${s.waiting}. Cancel it to use the switch.` : "",
      onclick: () => setEnabled(s.app_id, s.waiting ? s.waiting.startsWith("Turn CachyPlay off") : !s.enabled),
    }),
    s.waiting ? h("button", { class: "link small", onclick: () => cancelWaiting(s.app_id) }, "Cancel") : null);
}

/* ---------- progress track: where the game is on the way to working ---------- */
function track(d) {
  const ins = insightsFor(d.summary.app_id);
  const p = ins && ins.progress;
  if (!p || !["not_working", "testing", "seems_to_work", "works_with_fix"].includes(p.stage)) return null;
  const runs = runsOf(ins);
  const firstBad = [...runs].reverse().find((r) => r.outcome === "crash" || r.outcome === "problem");
  const at = { not_working: 1, testing: 2, seems_to_work: 2, works_with_fix: 3 }[p.stage];
  const fix = p.fix_label;
  const clean = `${Math.min(p.clean_runs, 3)} of 3 clean runs of ${duration(p.needed_secs)}`;
  const steps = [
    [at > 0 ? (firstBad && firstBad.outcome === "problem" ? "Had a problem" : "Crashed") : "Crashing",
      firstBad ? `${shortDay(firstBad.start)}: ${firstBad.text}` : "Found by CachyPlay"],
    [at > 1 ? "Tried a fix" : "Try a fix", at > 1 && fix ? fix : "You are here: pick the next one"],
    [at > 2 ? "Tested" : "Testing", at > 2 ? `${p.clean_runs} clean ${p.clean_runs === 1 ? "run" : "runs"}` : at === 2 ? clean : `3 clean runs of ${duration(p.needed_secs)}`],
    ["Works with a fix", at > 2 ? "Done. Nothing to do." : "Kept, and you can share it"],
  ];
  return h("ol", { class: "track", "aria-label": "Progress" }, steps.map(([title, detail], i) => {
    const cls = i < at ? "done" : i === at ? "now" : "";
    return h("li", { class: cls, "aria-current": i === at ? "step" : null },
      h("span", { class: "track-num" }, i < at || (at === 3 && i === 3) ? icon("check", 14) : String(i + 1)),
      h("span", { class: "track-text" }, h("strong", {}, title), h("span", {}, detail)));
  }));
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
    }, label, id === "fixes" && list && list.length && needsFixing(d) ? h("span", { class: "count" }, String(list.length)) : null)));
}

/* Something is wrong or being tested (not: working, or never had a problem). */
function needsFixing(d) {
  const ins = insightsFor(d.summary.app_id);
  const p = ins && ins.progress;
  if (p && ["not_working", "testing", "seems_to_work"].includes(p.stage)) return true;
  return !!(ins && ins.summary && ins.summary.light === "red");
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

/* ---------- Overview ---------- */
function overviewTab(d) {
  const s = d.summary;
  const ins = insightsFor(s.app_id);
  const list = ins ? ins.suggestions : undefined;
  const out = [];

  // "Did it work?" comes first: the answer makes everything else smarter.
  for (const x of isWorking(s.app_id) ? [] : (list || []).filter((x) => x.ask_verdict)) {
    out.push(card("ask",
      h("h2", {}, `Did "${x.title}" help?`),
      h("p", { class: "muted" }, `You've played ${s.name} since you applied it.`),
      h("div", { class: "actions" },
        h("button", { class: "primary", onclick: () => setVerdict(s.app_id, x.id, true) }, "Yes, it works now"),
        h("button", { class: "secondary", onclick: () => setVerdict(s.app_id, x.id, false) }, "No, still broken"))));
  }

  out.push(brief(d));
  const side = [upNextCard(d), ruledOutCard(d)].filter(Boolean);
  if (!side.length) side.push(pathCard(d));
  out.push(h("div", { class: "overview" }, startsChartCard(d), h("div", { class: "stack" }, ...side)));
  if (ins && ins.working && ins.working.length && ins.summary && ins.summary.light === "red") out.push(compareCard(ins));
  return out;
}

/* The mission brief: what happened, why, what to do, and one main button. For a game that works
   with a fix, the middle and right say where the fix lives and offer to keep it in Steam. */
function brief(d) {
  const s = d.summary;
  const ins = insightsFor(s.app_id);
  const sum = ins && ins.summary;
  if (!ins) {
    return card("brief", h("div", { class: "skeleton" }), h("div", { class: "skeleton short" }));
  }
  if (!sum) {
    return card("brief", h("p", { class: "muted" }, ins.error || "CachyPlay couldn't read this game's history."));
  }
  const blocks = [h("section", { class: "brief-block" }, h("h2", { class: "brief-label" }, "What happened"), h("p", { class: "brief-what" }, sum.what))];
  const p = ins.progress;
  const works = !!p && p.stage === "works_with_fix";
  // A fix typed into Steam by hand (like -dx12) lives in Steam only.
  const typed = p && p.steam_fix ? { by_cachyplay: false, in_steam: true, can_undo: false, launch_options: "", typed: p.steam_fix } : null;
  // A Proton switch lives in Steam's Compatibility setting, not in the launch options.
  const protonFix = !!p && !!p.fix_label && p.fix_label.startsWith("Switch to ");
  const home = typed || (protonFix ? null : d.fix_home);
  const showHome = home && (works || home.can_undo || (home.in_steam && !s.enabled && !!p && !!p.fix_label));
  // Nothing has gone wrong: no cause to explain and nothing to fix.
  const calm = ["grey", "green"].includes(sum.light) && !(p && ["not_working", "testing", "seems_to_work"].includes(p.stage));
  const testing = !!p && ["testing", "seems_to_work"].includes(p.stage);
  if (showHome) {
    blocks.push(fixHomeBlock(d, home), keepBlock(d, home));
  } else {
    if (sum.why && !calm) blocks.push(h("section", { class: "brief-block" }, h("h2", { class: "brief-label" }, testing ? "How it's tested" : "Why"), h("p", {}, sum.why)));
    blocks.push(h("section", { class: "brief-block todo-block" },
      h("h2", { class: "brief-label" }, "What to do"),
      h("p", {}, (!calm && sum.todo) || (s.enabled ? "Nothing right now. CachyPlay checks every start." : "Nothing right now.")),
      briefActions(d, sum, calm)));
  }
  const note = s.removed_in_steam && !s.waiting
    ? h("p", { class: "note put-back" }, h("strong", {}, "CachyPlay was removed from this game's launch options in Steam. "),
      "Editing launch options in Steam replaces the whole line, including CachyPlay's part. Turn the CachyPlay switch back on to put it back; your new options are kept. Tip: use \"Try these settings\" on the Settings tab instead, so Steam's launch options never change.")
    : null;
  return card("brief" + (showHome ? " works" : ""), note, h("div", { class: "brief-grid" }, ...blocks));
}

function reportButton(appId) {
  return h("button", { class: "secondary", onclick: () => openReport({ appId }), title: "Save a bug report for this game" }, "Report a problem");
}

function briefActions(d, sum, calm) {
  const s = d.summary;
  const report = reportButton(s.app_id);
  if (sum.fix_id && !calm) {
    return h("div", { class: "hero-actions" },
      h("button", { class: "primary big", disabled: state.busy, onclick: (e) => fixIt(s.app_id, e.currentTarget) }, "Fix it"),
      report,
      h("span", { class: "fix-caption" }, `Fix it tries: ${sum.fix_label}. You can undo it.`));
  }
  return h("div", { class: "hero-actions" }, report);
}

function fixHomeBlock(d, home) {
  const row = (label, on, onText, offText) => h("li", {},
    h("span", {}, label),
    h("span", { class: on ? "where-on" : "where-off" }, h("span", { class: `light ${on ? "green" : "grey"}`, "aria-hidden": "true" }), on ? onText : offText));
  return h("section", { class: "brief-block" },
    h("h2", { class: "brief-label" }, "Where the fix lives"),
    h("ul", { class: "where" },
      row("Added by CachyPlay", home.by_cachyplay, "Now", d.summary.enabled ? "Not for this fix" : "Off"),
      row("In your Steam launch options", home.in_steam, "Now", "Not yet")),
    home.typed ? h("p", { class: "muted" }, "You typed ", optionsCode(home.typed, "tech"), " there yourself.") : null);
}

function keepBlock(d, home) {
  const s = d.summary;
  const report = reportButton(s.app_id);
  if (home.can_undo) {
    const both = home.by_cachyplay;
    return h("section", { class: "brief-block todo-block" },
      h("h2", { class: "brief-label" }, "Kept in Steam"),
      h("p", {}, both
        ? `The fix is in Steam's launch options, and CachyPlay is on too (it never adds the same fix twice). Undo takes the fix back out of Steam.`
        : `Steam adds the fix itself, so ${s.name} works with CachyPlay off. Undo puts everything back the way it was.`),
      h("div", { class: "hero-actions" },
        h("button", { class: "secondary", disabled: !!editBlocked() && !canQueueNow(), onclick: () => keepInSteam(s.app_id, false) }, canQueueNow() ? "Undo when Steam closes" : "Undo"),
        report));
  }
  if (home.in_steam) {
    return h("section", { class: "brief-block todo-block" },
      h("h2", { class: "brief-label" }, "What to do"),
      h("p", {}, `Nothing. Steam adds the fix itself, so ${s.name} works with CachyPlay on or off.`),
      h("div", { class: "hero-actions" }, report));
  }
  const waiting = !!s.waiting && s.waiting.startsWith("Keep ");
  return h("section", { class: "brief-block todo-block" },
    h("h2", { class: "brief-label" }, "Want it without CachyPlay?"),
    h("p", {}, waiting
      ? "Waiting for Steam to close. The fix goes into Steam's launch options the moment it exits."
      : `Keep the fix in Steam. ${s.name} keeps working with CachyPlay off. Undo puts it back.`),
    h("div", { class: "hero-actions" },
      waiting
        ? h("button", { class: "secondary", onclick: () => cancelWaiting(s.app_id) }, "Cancel")
        : h("button", { class: "primary big", disabled: (!!editBlocked() && !canQueueNow()) || !!s.waiting, title: editBlocked() && !canQueueNow() ? editBlocked() : "", onclick: () => keepInSteam(s.app_id, true) }, canQueueNow() ? "Keep in Steam when it closes" : "Keep this fix in Steam"),
      report),
    h("details", { class: "tech-details" }, h("summary", {}, "What Steam's launch options become"), optionsCode(home.launch_options, "tech")));
}

/* ---------- recent starts as a bar chart ---------- */
const SVG_NS = "http://www.w3.org/2000/svg";
function svg(tag, attrs = {}, ...kids) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, String(v));
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const BAR_CLASS = { fine: "bar-good", crash: "bar-bad", problem: "bar-bad", short: "bar-warn", didnt_start: "bar-warn", running: "bar-live", ok: "bar-muted" };

function openRun(start) {
  state.openRuns.add(start);
  switchTab("history");
  const row = [...document.querySelectorAll("[data-run]")].find((li) => li.dataset.run === start);
  if (row) row.scrollIntoView({ block: "center" });
}

function monthDay(iso) {
  const t = new Date(iso);
  return isNaN(t) ? iso : t.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function startsChartCard(d) {
  const ins = insightsFor(d.summary.app_id);
  const items = ((ins && ins.timeline) || []).slice().reverse(); // oldest first
  const runs = items.filter((it) => it.kind === "run").slice(-14);
  if (!runs.length) {
    return card("grow chart-card",
      h("h2", { class: "card-title" }, "Recent starts"),
      h("p", { class: "muted" }, ins ? (d.summary.enabled ? "Start the game from Steam and it shows up here." : "Starts show up here once you play the game from Steam.") : "Loading…"));
  }
  const SLOT = 46, TOP = 22, H = 96, W = runs.length * SLOT + 16;
  let lastLabel = -Infinity;
  const maxSecs = Math.max(60, ...runs.map((r) => r.secs || 0));
  const first = new Date(runs[0].start);
  const kids = [];
  // Update markers between bars: fixes, Proton switches, driver and game updates.
  const marks = items.filter((it) => it.kind === "event" && new Date(it.at) >= first && it.what !== "off");
  for (const m of marks) {
    const t = new Date(m.at);
    let i = runs.findIndex((r) => new Date(r.start) > t);
    if (i < 0) i = runs.length;
    const x = i * SLOT + 2;
    kids.push(svg("g", { class: `mark mark-${m.what}` },
      svg("title", {}, `${when(m.at)}: ${m.text}`),
      svg("line", { x1: x, x2: x, y1: TOP - 6, y2: TOP + H }),
      svg("path", { d: `M${x - 5} ${TOP - 14} L${x + 5} ${TOP - 14} L${x} ${TOP - 6} Z` })));
  }
  runs.forEach((r, i) => {
    const secs = r.secs == null ? (r.running ? maxSecs / 3 : 0) : r.secs;
    const bh = Math.max(6, Math.round((secs / maxSecs) * H));
    const x = i * SLOT + 8;
    const off = !r.through_cachyplay;
    const label = `${when(r.start)}: ${r.text}${off ? (r.fix_in_steam ? " (CachyPlay off, fix in Steam)" : " (CachyPlay off)") : ""}`;
    kids.push(svg("g", {
      class: `bar ${BAR_CLASS[r.outcome] || "bar-muted"}${off ? " bar-off" : ""}`, tabindex: "0", role: "button", "aria-label": label,
      onclick: () => openRun(r.start), onkeydown: (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openRun(r.start); } },
    }, svg("title", {}, label),
      // The whole column is the click target, so a short run is as easy to hit as a long one.
      svg("rect", { class: "hit", x: x - 4, y: TOP, width: SLOT - 4, height: H }),
      svg("rect", { x, y: TOP + H - bh, width: SLOT - 12, height: bh, rx: 3 })));
    const prev = runs[i - 1];
    // A day's label under its first start, skipped when it would run into the one before.
    // Each day is labelled under its first start (short dates fit one column each).
    if ((!prev || new Date(prev.start).toDateString() !== new Date(r.start).toDateString()) && x - lastLabel >= SLOT) {
      kids.push(svg("text", { class: "bar-day", x: x - 2, y: TOP + H + 16 }, monthDay(r.start)));
      lastLabel = x;
    }
  });
  const chart = svg("svg", { class: "starts-chart", width: W, height: TOP + H + 24, viewBox: `0 0 ${W} ${TOP + H + 24}`, role: "group", "aria-label": "Recent starts, oldest first" }, ...kids);
  const anyOff = runs.some((r) => !r.through_cachyplay);
  return card("grow chart-card",
    h("div", { class: "chart-head" }, h("h2", { class: "card-title" }, "Recent starts"), h("span", { class: "sys-detail" }, "Bar height = minutes played")),
    h("div", { class: "chart-scroll" }, chart),
    h("ul", { class: "legend" },
      h("li", {}, h("span", { class: "key bar-good" }), "Ran fine"),
      h("li", {}, h("span", { class: "key bar-bad" }), "Crashed"),
      runs.some((r) => BAR_CLASS[r.outcome] === "bar-warn") ? h("li", {}, h("span", { class: "key bar-warn" }), "Closed early") : null,
      anyOff ? h("li", {}, h("span", { class: "key bar-good bar-off" }), "CachyPlay off") : null,
      marks.some((m) => m.what !== "driver") ? h("li", {}, h("span", { class: "key mark-key" }), "Fix or game update") : null,
      marks.some((m) => m.what === "driver") ? h("li", {}, h("span", { class: "key mark-key driver" }), "Driver update") : null,
      h("li", { class: "muted" }, "Click a bar for that start's details.")));
}

/* ---------- Up next / Ruled out ---------- */
function upNextCard(d) {
  const appId = d.summary.app_id;
  const list = suggestionsFor(appId);
  if (!list) return null;
  const ins = insightsFor(appId);
  const sum = ins?.summary;
  // A game that works (with or without a fix) has nothing up next.
  if (isWorking(appId)) return null;
  if (sum && ["grey", "green"].includes(sum.light) && !(ins?.progress && ["not_working", "testing", "seems_to_work"].includes(ins.progress.stage))) return null;
  const next = list.filter((x) => x.status === "new" && x.id !== (sum && sum.fix_id)).slice(0, 3);
  if (!next.length) return null;
  const blocked = editBlocked();
  return card("up-next",
    h("h2", { class: "card-title" }, "Up next"),
    h("ul", { class: "next-list" }, next.map((x) => {
      let btn;
      if (x.fix) btn = h("button", { class: "secondary small", onclick: (e) => applyFix(appId, x.id, e.currentTarget) }, x.try_again ? "Try again" : "Try");
      else if (x.switch_tool && !x.switched) btn = h("button", { class: "secondary small", disabled: !!blocked && !canQueueNow(), title: blocked || "", onclick: () => compatAction("switch_compat_tool", { appId, name: x.switch_tool.name }) }, "Try");
      else btn = h("button", { class: "link", onclick: () => switchTab("fixes") }, "How");
      return h("li", {},
        h("span", { class: "next-text" }, h("strong", {}, x.title),
          h("span", { class: "muted" }, [CONFIDENCE[x.confidence], x.big_step ? "big step, undoable" : x.fix || x.switch_tool ? "one click, undoable" : null].filter(Boolean).join(" · "))),
        btn);
    })),
    list.filter((x) => x.status === "new").length > next.length + (sum && sum.fix_id ? 1 : 0)
      ? h("button", { class: "link", onclick: () => switchTab("fixes") }, "See every suggestion") : null);
}

function ruledOutCard(d) {
  const ins = insightsFor(d.summary.app_id);
  if (!ins) return null;
  const names = [...((ins.find_cause && ins.find_cause.ruled_out) || []), ...(ins.suggestions || []).filter((x) => x.status === "didnt_help").map((x) => x.title)];
  const seen = new Set();
  const unique = names.filter((n) => { const k = n.toLowerCase(); if (seen.has(k)) return false; seen.add(k); return true; });
  if (!unique.length) return null;
  return card("ruled-out",
    h("h2", { class: "card-title" }, "Ruled out ", h("span", { class: "count" }, String(unique.length))),
    h("ul", { class: "chips" }, unique.map((n) => h("li", { class: "chip muted" }, n))),
    h("button", { class: "link", onclick: () => switchTab("fixes") }, "Each one can be tried again on the Fixes tab"));
}

function pathCard(d) {
  return card("",
    h("h2", { class: "card-title" }, "How the game starts"),
    h("div", { class: "path" }, d.pipeline.map((n) =>
      h("div", { class: `node ${n.state}` }, h("span", { class: "ring" }), h("strong", {}, n.label), h("span", {}, n.detail)))));
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
  return h("li", { class: "tl-run", "data-run": r.start },
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
