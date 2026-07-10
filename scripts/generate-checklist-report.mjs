import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const checklistPath = path.join(root, "quality", "acceptance-checklist.json");
const reportPath = path.join(root, "CHECKLIST_REPORT.md");

const checklist = JSON.parse(fs.readFileSync(checklistPath, "utf8"));

function existsMatchingFile(item) {
  const target = path.join(root, item.path || "");
  if (!fs.existsSync(target)) {
    return false;
  }

  if (item.filename) {
    return fs.existsSync(path.join(target, item.filename));
  }

  if (item.extension) {
    return fs.readdirSync(target).some((name) => name.endsWith(item.extension));
  }

  return true;
}

function statusFor(item) {
  if (item.env) {
    const value = process.env[item.env];
    if (value === "success") return { icon: "PASS", state: "passed", detail: item.env };
    if (value === "failure") return { icon: "FAIL", state: "failed", detail: item.env };
    if (value === "cancelled") return { icon: "FAIL", state: "cancelled", detail: item.env };
    if (value === "skipped") return { icon: "SKIP", state: "skipped", detail: item.env };
    if (value) return { icon: "INFO", state: value, detail: item.env };
  }

  if (item.method === "artifact_check") {
    return existsMatchingFile(item)
      ? { icon: "PASS", state: "passed", detail: item.path }
      : { icon: "FAIL", state: "missing", detail: item.path };
  }

  if (item.auto) {
    return { icon: "TODO", state: "not wired", detail: item.method };
  }

  return { icon: "MANUAL", state: "manual", detail: item.manual_reason || item.method };
}

const rows = checklist.map((item) => ({ item, status: statusFor(item) }));
const failed = rows.filter((row) => ["failed", "cancelled", "missing"].includes(row.status.state));
const manual = rows.filter((row) => row.status.state === "manual");
const passed = rows.filter((row) => row.status.state === "passed");

const lines = [];
lines.push("# Speakyfi Windows Acceptance Report");
lines.push("");
lines.push(`Generated: ${new Date().toISOString()}`);
lines.push("");
lines.push("## Summary");
lines.push("");
lines.push(`- Passed: ${passed.length}`);
lines.push(`- Failed or missing: ${failed.length}`);
lines.push(`- Manual checks: ${manual.length}`);
lines.push("");
lines.push("## Checklist");
lines.push("");
lines.push("| Status | ID | Check | Method | Detail |");
lines.push("|---|---|---|---|---|");

for (const row of rows) {
  const { item, status } = row;
  lines.push(`| ${status.icon} | \`${item.id}\` | ${item.title} | ${item.method} | ${status.detail || ""} |`);
}

if (manual.length > 0) {
  lines.push("");
  lines.push("## Manual Checks For Ivan");
  lines.push("");
  for (const row of manual) {
    lines.push(`- \`${row.item.id}\`: ${row.item.title}`);
    lines.push(`  Reason: ${row.item.manual_reason || "Needs manual verification."}`);
  }
}

if (failed.length > 0) {
  lines.push("");
  lines.push("## Failed Checks");
  lines.push("");
  for (const row of failed) {
    lines.push(`- \`${row.item.id}\`: ${row.item.title} (${row.status.state})`);
  }
}

lines.push("");
fs.writeFileSync(reportPath, lines.join("\n"), "utf8");
console.log(fs.readFileSync(reportPath, "utf8"));

if (process.env.GITHUB_STEP_SUMMARY) {
  fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, fs.readFileSync(reportPath, "utf8"));
}
