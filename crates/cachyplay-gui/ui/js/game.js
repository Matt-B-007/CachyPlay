// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// A game's page: the status, What happened / Why / What to do, the tabs.

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

/* The status chip's colour always matches its words: "Not working" is red, testing or working
   with a fix is amber, runs fine is green (the game list's light can lag behind the status). */
const STAGE_LIGHT = { not_working: "red", testing: "yellow", seems_to_work: "yellow", works_with_fix: "yellow", runs_fine: "green" };
function stageLight(d, light) {
  const p = insightsFor(d.summary.app_id)?.progress;
  return (p && STAGE_LIGHT[p.stage]) || light;
}

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
        h("span", { class: `light-chip ${stageLight(d, light)}` }, h("span", { class: `light ${stageLight(d, light)}` }), stageLabel(d) || LIGHT_LABEL[light]),
        rating ? h("button", { class: `protondb ${rating.tier}`, title: `${rating.meaning} (${rating.reports} reports on ProtonDB)`, onclick: () => openUrl(rating.url) }, `ProtonDB: ${rating.tier}`) : null),
      ...lines,
      h("div", { class: "hero-actions" },
        primary,
        h("button", { class: primary ? "secondary big" : "primary big", onclick: () => openReport({ appId: s.app_id }), title: "Save a bug report for this game" }, "Report a problem"),
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
