// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Suggested fixes, ruled-out tries and what's applied right now.

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

/* Resets offer Undo straight away in their message (House Standard). */
const UNDO_OF = { reset_shader_cache: "undo_shader_cache" };

async function simpleAction(cmd, appId) {
  try {
    const undo = UNDO_OF[cmd];
    toast(await invoke(cmd, { appId }), "success", undo ? { label: "Undo", run: () => simpleAction(undo, appId) } : undefined);
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
        h("button", { class: "primary", onclick: () => openReport({ appId: d.summary.app_id, destination: "nvidia", description: ins.nvidia_report, summary: `${d.summary.name}: driver error (Xid) keeps happening` }) }, "Tell NVIDIA…"),
        h("button", { class: "secondary", onclick: () => copyText(ins.nvidia_report, "Copied. Paste it into your post.") }, "Copy text"))));
  }
  if (ins && ins.developer_report) {
    out.push(card("dev",
      h("h2", { class: "card-title" }, "Tell the developer"),
      h("p", { class: "muted" }, "Developers fix what they can see. Copy this, post it in the game's Steam discussions (or the developer's Discord), and attach a bug report."),
      h("pre", { class: "dev-text" }, ins.developer_report),
      h("div", { class: "actions" },
        h("button", { class: "primary", onclick: () => openReport({ appId: d.summary.app_id, destination: "steam", description: ins.developer_report, discussions: ins.discussions_url, summary: `${d.summary.name} problem on Linux (Proton)` }) }, "Tell the developer…"),
        h("button", { class: "secondary", onclick: () => copyText(ins.developer_report, "Copied. Paste it into your post.") }, "Copy text"))));
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
