// SPDX-License-Identifier: GPL-3.0-or-later
"use strict";
// The Report a problem dialog and saved bug reports.

/* ---------- Report a problem (House Standard dialog) ----------
   Main buttons: Save to file, Copy report, Open log folder. Secondary: where to send it (GitHub,
   NVIDIA's forum, the game's Steam discussions). Nothing is sent by CachyPlay itself. */
state.report = null;

function saveReport(appId) {
  openReport({ appId });
}

async function openReport(opts = {}) {
  closeMenus();
  const appId = opts.appId || null;
  const game = appId && state.overview ? state.overview.games.find((g) => g.app_id === appId) : null;
  state.report = { ...opts, appId, game, saved: null };
  $("report-for").textContent = game
    ? `For ${game.name}: the report includes its logs, crashes and what was tried.`
    : "A general report about CachyPlay and this PC. For a problem with one game, open that game first.";
  $("report-summary").value = opts.summary || "";
  $("report-description").value = opts.description || (opts.problem ? `${opts.problem}\n\nWhat I expected: ` : "");
  let header = "";
  try {
    header = await invoke("report_header");
  } catch {
    header = "";
  }
  state.report.header = header;
  $("report-preview").textContent = `${header}\nAlso included: ${game ? "this game's crashes, starts, fixes and logs (Proton, Steam), " : ""}system details (graphics card, driver, kernel), CachyPlay's log and settings, and recent driver messages.`;
  renderDestinations();
  $("report-dialog").showModal();
  $("report-summary").focus();
}

function renderDestinations() {
  const r = state.report;
  const info = state.appInfo || {};
  const out = [h("p", { class: "muted" }, "Send it somewhere (opens in your browser; attach the saved file there):")];
  const row = h("div", { class: "row-actions" });
  row.append(h("button", { type: "button", class: "secondary", onclick: () => sendTo("github") }, icon("external"), " Open on GitHub"));
  if (r.destination === "nvidia" || (insightsFor(r.appId) || {}).nvidia_report) {
    row.append(h("button", { type: "button", class: r.destination === "nvidia" ? "primary" : "secondary", onclick: () => sendTo("nvidia") }, icon("external"), " NVIDIA forum"));
  }
  if (r.destination === "steam" || r.discussions) {
    row.append(h("button", { type: "button", class: r.destination === "steam" ? "primary" : "secondary", onclick: () => sendTo("steam") }, icon("external"), " Steam discussions"));
  }
  out.push(row);
  out.push(h("p", { class: "hint-small" }, info.repo_private
    ? "GitHub: only the owner can see this repository while it's private."
    : "GitHub: this creates a PUBLIC issue that anyone can read."));
  $("report-destinations").replaceChildren(...out);
}

/* Save once per dialog; the other buttons reuse the saved file. */
async function ensureSaved() {
  const r = state.report;
  if (r.saved) return r.saved;
  const progress = toast("Saving bug report…");
  try {
    r.saved = await invoke("create_report", { appId: r.appId, summary: $("report-summary").value, description: $("report-description").value });
    return r.saved;
  } catch (e) {
    toast(`Couldn't save the bug report: ${e}`, "error", { label: "Try again", run: ensureSaved });
    return null;
  } finally {
    progress.remove();
  }
}

async function reportSave() {
  const saved = await ensureSaved();
  if (!saved) return;
  toast(`Saved ${saved.name}`, "success", { label: "Show in folder", run: () => invoke("open_folder", { path: saved.folder }) });
  renderReports();
}

async function reportCopy() {
  const saved = await ensureSaved();
  if (!saved) return;
  await copyText(saved.text, `Copied. The full report (with logs) is ${saved.name}.`);
  renderReports();
}

async function reportLogs() {
  try {
    const f = await invoke("report_folders");
    await invoke("open_folder", { path: f.logs });
  } catch (e) {
    toast(String(e), "error", { label: "OK", run: () => {} });
  }
}

async function sendTo(where) {
  const saved = await ensureSaved();
  if (!saved) return;
  const r = state.report;
  const summary = $("report-summary").value.trim() || (r.game ? `Problem with ${r.game.name}` : "Problem with CachyPlay");
  const said = $("report-description").value.trim();
  if (where === "github") {
    const body = `${said || "(describe what happened)"}\n\n${r.header.trim()}\n\nPlease attach the saved report: ${saved.name} (in ${saved.folder}).`;
    const repo = (state.appInfo && state.appInfo.repo) || "Matt-B-007/CachyPlay";
    await openUrl(`https://github.com/${repo}/issues/new?title=${encodeURIComponent(summary)}&body=${encodeURIComponent(body)}`);
  } else {
    await copyText(`${summary}\n\n${said}`, `Copied. Paste it into your post and attach ${saved.name}.`);
    await openUrl(where === "nvidia" ? "https://forums.developer.nvidia.com/c/gpu-graphics/linux/148" : r.discussions);
  }
  toast(`Attach ${saved.name} to your post.`, "info", { label: "Show in folder", run: () => invoke("open_folder", { path: saved.folder }) });
  renderReports();
}

/* Saved bug reports, listed in Settings › Privacy. */
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
    list.replaceChildren(h("li", { class: "empty-row" }, "No bug reports saved yet. Help › Report a problem saves one to your Desktop."));
    return;
  }
  list.replaceChildren(...reports.map((r) => h("li", {},
    h("div", { class: "what" },
      h("strong", {}, r.game || (r.app_id ? `Game ${r.app_id}` : "General report")),
      h("span", {}, `Saved ${when(r.created.replace(" ", "T"))}, ${r.size_kb} KB`)),
    h("button", { type: "button", class: "secondary", onclick: () => invoke("open_folder", { path: r.path }) }, "Show in folder"))));
}
