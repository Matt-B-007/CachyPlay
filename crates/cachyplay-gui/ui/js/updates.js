// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// Updates: checking, the update dialog, going back.

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
  // Library shows the update state too.
  if (!state.selected && state.overview && $("settings").hidden) renderHome();
}

function renderUpdate() {
  const u = state.update;
  const pill = $("update-pill");
  const line = $("update-line");
  const btn = $("update-now-settings");
  if (!u) return;
  for (const r of document.querySelectorAll('input[name="channel"]')) r.checked = r.value === u.channel;
  $("roll-back").hidden = !u.can_roll_back;
  $("last-checked").textContent = u.last_checked ? `Last checked ${when(u.last_checked)}.` : "Not checked yet.";
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
    btn.onclick = () => openUpdateDialog({ toStable: true });
  } else if (u.skipped && u.latest) {
    pill.hidden = true;
    line.className = "status-line";
    line.textContent = `You chose to skip ${u.latest.tag}. You have v${u.current}; the next version will be offered.`;
    btn.hidden = false;
    btn.textContent = `Install ${u.latest.tag} anyway`;
    btn.onclick = async () => {
      await invoke("skip_version", { tag: null }).catch(() => {});
      await checkUpdates(false);
      openUpdateDialog();
    };
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

/* The House Standard update dialog: "CachyPlay X is available" (+ Beta tag), "You have X",
   release notes, Update now · Later · Skip this version; then progress and Restart. */
function openUpdateDialog(opts = {}) {
  const u = state.update;
  const rel = opts.toStable ? u && u.stable_now : u && u.latest;
  if (!rel || (!opts.toStable && !u.available)) return;
  state.updateTarget = { rel, toStable: !!opts.toStable };
  $("update-title").textContent = `CachyPlay ${rel.tag.replace(/^v/, "")} is available`;
  $("update-beta").hidden = !rel.prerelease;
  $("update-sub").textContent = `You have ${u.current}.`;
  const stableNote = $("update-stable-note");
  stableNote.hidden = !opts.toStable;
  stableNote.textContent = opts.toStable ? `You're on beta ${u.current}. This installs the newest normal version.` : "";
  $("update-notes").replaceChildren(...notesView(rel.notes || ""));
  $("update-progress").hidden = true;
  $("update-command").hidden = true;
  const now = $("update-now");
  now.hidden = false;
  now.disabled = false;
  now.textContent = "Update now";
  now.onclick = updateNow;
  $("update-later").hidden = false;
  $("update-later").textContent = "Later";
  $("update-skip").hidden = !!opts.toStable;
  if (u.kind === "package") {
    $("update-sub").textContent = `You have ${u.current}. CachyPlay was installed from the AUR, so your package manager updates it:`;
    $("update-command").hidden = false;
    $("update-command-text").value = u.command;
    now.hidden = true;
  } else if (u.kind === "notinstalled") {
    $("update-sub").textContent = `You have ${u.current}, running from a download or build folder. Install CachyPlay first, then update.`;
    now.textContent = "Install CachyPlay";
    now.onclick = async () => {
      $("update-dialog").close();
      await installSelf();
    };
  }
  $("update-dialog").showModal();
}

async function skipThisVersion() {
  const t = state.updateTarget;
  if (!t) return;
  try {
    await invoke("skip_version", { tag: t.rel.tag });
    toast(`${t.rel.tag} won't be offered again. You'll hear about the next version.`, "success", {
      label: "Undo", run: async () => { await invoke("skip_version", { tag: null }); checkUpdates(true); } });
  } catch (e) {
    toast(String(e), "error");
  }
  $("update-dialog").close();
  checkUpdates(true);
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
      list.append(h("li", {}, ...proseWithCode(bullet[1].replace(/\*\*/g, ""))));
      continue;
    }
    list = null;
    if (!line) continue;
    const head = line.match(/^#{1,6}\s+(.*)/);
    out.push(head ? h("h4", {}, head[1]) : h("p", {}, ...proseWithCode(line.replace(/\*\*/g, ""))));
  }
  return out;
}

/* `-dx12` in notes becomes a code word that never splits after its hyphen. */
function proseWithCode(text) {
  return text.split(/`([^`]+)`/).map((part, i) => (i % 2 ? h("code", { class: "tok" }, part) : part));
}

async function updateNow() {
  const t = state.updateTarget;
  const now = $("update-now");
  now.disabled = true;
  $("update-skip").hidden = true;
  $("update-later").hidden = true;
  $("update-progress").hidden = false;
  $("update-progress").classList.remove("done");
  $("update-progress-text").textContent = "Downloading the update and checking its signature…";
  try {
    await invoke("install_update", { toStable: t && t.toStable });
    $("update-progress").classList.add("done");
    $("update-progress-text").textContent = "Update installed. Restart to use it.";
    state.update.available = false;
    state.update.can_roll_back = true;
    renderUpdate();
    now.disabled = false;
    now.textContent = "Restart";
    now.onclick = () => invoke("restart_app");
    $("update-later").hidden = false;
    $("update-later").textContent = "Restart later";
  } catch (e) {
    $("update-progress").hidden = true;
    $("update-later").hidden = false;
    now.disabled = false;
    now.textContent = "Try again";
    toast(`The update wasn't installed: ${e} You still have ${state.update.current}.`, "error");
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

function openSettings(section) {
  closeMenus();
  const o = state.overview;
  if (!o) return;
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
  renderReports();
  if (!state.update) checkUpdates(false);
  renderKept();
  renderObsSettings();
  renderAppSettings();
  if (!$("pc-checks").children.length) runPcCheck();
  $("games-view").hidden = true;
  $("settings").hidden = false;
  $("settings-content").scrollTop = 0;
  if (section && $(section)) $(section).scrollIntoView({ block: "start" });
  $("close-settings").focus();
}

function closeSettings() {
  $("settings").hidden = true;
  $("games-view").hidden = false;
}
