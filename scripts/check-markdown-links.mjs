#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = path.resolve(process.argv[2] || process.cwd());
const ignoredDirectories = new Set([".git", "node_modules", "target"]);
const markdownFiles = [];

function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) continue;
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(absolute);
    else if (entry.isFile() && entry.name.toLowerCase().endsWith(".md")) {
      markdownFiles.push(absolute);
    }
  }
}

function withoutCode(markdown) {
  return markdown
    .replace(/```[\s\S]*?```/g, "")
    .replace(/~~~[\s\S]*?~~~/g, "")
    .replace(/`[^`\n]*`/g, "");
}

function localTarget(rawTarget) {
  let target = rawTarget.trim();
  if (target.startsWith("<") && target.endsWith(">")) {
    target = target.slice(1, -1);
  }
  if (
    !target ||
    target.startsWith("#") ||
    target.startsWith("//") ||
    /^[a-z][a-z0-9+.-]*:/i.test(target)
  ) {
    return null;
  }

  target = target.split("#", 1)[0].split("?", 1)[0];
  try {
    return decodeURIComponent(target);
  } catch {
    return target;
  }
}

function hasExactCase(absolute) {
  const relative = path.relative(root, absolute);
  if (relative.startsWith("..") || path.isAbsolute(relative)) return false;

  let current = root;
  for (const segment of relative.split(path.sep).filter(Boolean)) {
    let entries;
    try {
      entries = fs.readdirSync(current);
    } catch {
      return false;
    }
    if (!entries.includes(segment)) return false;
    current = path.join(current, segment);
  }
  return fs.existsSync(current);
}

function resolveTarget(sourceFile, target) {
  const base = target.startsWith("/") ? root : path.dirname(sourceFile);
  return path.resolve(base, target.replace(/^\/+/, ""));
}

walk(root);

const failures = [];
let checkedLinks = 0;
for (const file of markdownFiles.sort()) {
  const source = withoutCode(fs.readFileSync(file, "utf8"));
  const targets = [];
  const patterns = [
    /!?\[[^\]]*\]\((<[^>]+>|[^)\s]+)(?:\s+["'][^"']*["'])?\)/g,
    /^\s*\[[^\]]+\]:\s*(<[^>]+>|\S+)/gm,
    /\b(?:href|src)=["']([^"']+)["']/gi,
  ];

  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) targets.push(match[1]);
  }

  for (const rawTarget of targets) {
    const target = localTarget(rawTarget);
    if (!target) continue;
    checkedLinks += 1;
    const resolved = resolveTarget(file, target);
    if (!hasExactCase(resolved)) {
      failures.push(
        `${path.relative(root, file)} -> ${rawTarget} (missing ${path.relative(root, resolved)})`,
      );
    }
  }
}

if (failures.length > 0) {
  process.stderr.write(`Broken local Markdown links (${failures.length}):\n`);
  process.stderr.write(`${failures.join("\n")}\n`);
  process.exit(1);
}

process.stdout.write(
  `Markdown links OK: ${markdownFiles.length} files, ${checkedLinks} local targets.\n`,
);
