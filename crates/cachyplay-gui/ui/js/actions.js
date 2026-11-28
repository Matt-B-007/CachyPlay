// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Things the buttons do: turning CachyPlay on or off, resets, Proton, Steam.

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
    } else if (cmd === "reset_prefix") {
      toast(await invoke(cmd, { appId }), "success", { label: "Undo", run: () => prefixAction("undo_prefix_reset", appId, name) });
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

/* Launch options, each option kept whole: "-nosplash" never wraps after its hyphen. */
function optionsCode(text, cls) {
  const words = String(text || "").split(/( +)/);
  return h("code", { class: cls }, words.map((w) => (w.trim() ? h("span", { class: "tok" }, w) : " ")));
}

function isWorking(appId) {
  const p = insightsFor(appId)?.progress;
  return !!p && ["works_with_fix", "runs_fine"].includes(p.stage);
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
    toast(await invoke("set_enabled", { appId, enabled: on }), "success",
      on ? undefined : { label: "Undo", run: () => setEnabled(appId, true) });
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    await load();
  }
}

/* "Keep this fix in Steam" (keep = true) and its Undo. Waits for Steam to close if it's open. */
async function keepInSteam(appId, keep) {
  state.busy = true;
  try {
    if (canQueueNow()) {
      toast(await invoke("queue_action", { action: { action: keep ? "keep_in_steam" : "undo_keep_in_steam", app_id: appId } }), "success", { label: "Close Steam for me", run: closeSteam });
      return;
    }
    toast(await invoke(keep ? "keep_in_steam" : "undo_keep_in_steam", { appId }), "success",
      keep ? { label: "Undo", run: () => keepInSteam(appId, false) } : undefined);
  } catch (e) {
    toast(String(e), "error");
  } finally {
    state.busy = false;
    state.insights.delete(appId);
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
