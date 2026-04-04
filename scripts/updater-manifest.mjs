#!/usr/bin/env node
/**
 * CI helpers for Tauri v2 static updater JSON (GitHub Releases).
 *   node scripts/updater-manifest.mjs partial-windows [release|debug]
 *   node scripts/updater-manifest.mjs partial-darwin [release|debug]
 *   node scripts/updater-manifest.mjs merge <windows-partial.json> <darwin-partial.json> -o latest.json
 *
 * Env: GITHUB_REF_NAME or YAPPER_RELEASE_TAG for download URLs (e.g. v1.0.8).
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

function readJson(p) {
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

function readConf() {
  return readJson(path.join(ROOT, "src-tauri", "tauri.conf.json"));
}

function parseGithubRepo(conf) {
  const ep = conf.plugins?.updater?.endpoints?.[0];
  if (!ep || typeof ep !== "string") {
    throw new Error("tauri.conf.json: missing plugins.updater.endpoints[0]");
  }
  const m = ep.match(/github\.com\/([^/]+)\/([^/]+)/);
  if (!m) {
    throw new Error(`Could not parse owner/repo from updater endpoint: ${ep}`);
  }
  return { owner: m[1], repo: m[2] };
}

function releaseTag(version) {
  const t = process.env.GITHUB_REF_NAME || process.env.YAPPER_RELEASE_TAG;
  if (t) return t;
  return version;
}

function assetUrl(owner, repo, tag, fileName) {
  return `https://github.com/${owner}/${repo}/releases/download/${tag}/${fileName}`;
}

function readSig(filePath) {
  const sigPath = `${filePath}.sig`;
  if (!fs.existsSync(sigPath)) {
    throw new Error(`Missing signature: ${sigPath}`);
  }
  return fs.readFileSync(sigPath, "utf8");
}

function partialWindows(profile) {
  const conf = readConf();
  const version = conf.version;
  if (!version) throw new Error("tauri.conf.json missing version");

  const { owner, repo } = parseGithubRepo(conf);
  const tag = releaseTag(version);
  const tauriDir = path.join(ROOT, "src-tauri");
  const nsisDir = path.join(tauriDir, "target", profile, "bundle", "nsis");
  const msiDir = path.join(tauriDir, "target", profile, "bundle", "msi");

  if (!fs.existsSync(nsisDir)) {
    throw new Error(`NSIS bundle folder not found: ${nsisDir}`);
  }

  const nsisFiles = fs.readdirSync(nsisDir).filter((f) => f.endsWith(".exe") && !f.endsWith(".sig"));
  let exe = nsisFiles.find((f) => f === `Yapper_${version}_x64-setup.exe`);
  if (!exe) {
    exe = nsisFiles.find((f) => f.endsWith("_x64-setup.exe"));
  }
  if (!exe) {
    throw new Error(`No *-setup.exe under ${nsisDir}`);
  }

  const exePath = path.join(nsisDir, exe);
  const nsisSig = readSig(exePath);
  const nsisUrl = assetUrl(owner, repo, tag, exe);

  /** @type {Record<string, { signature: string; url: string }>} */
  const platforms = {};

  let msi = null;
  if (fs.existsSync(msiDir)) {
    const msiFiles = fs.readdirSync(msiDir).filter((f) => f.endsWith(".msi"));
    msi = msiFiles.find((f) => f.startsWith(`Yapper_${version}_`)) || msiFiles[0];
  }
  if (msi) {
    const msiPath = path.join(msiDir, msi);
    platforms["windows-x86_64-msi"] = {
      signature: readSig(msiPath),
      url: assetUrl(owner, repo, tag, msi),
    };
  }

  platforms["windows-x86_64-nsis"] = { signature: nsisSig, url: nsisUrl };
  platforms["windows-x86_64"] = { signature: nsisSig, url: nsisUrl };

  const out = { platforms };
  fs.writeFileSync(path.join(ROOT, "platforms.partial.json"), JSON.stringify(out, null, 2) + "\n", "utf8");
  console.log("Wrote platforms.partial.json (windows)");
}

function darwinPlatformKey(fileName) {
  const lower = fileName.toLowerCase();
  if (lower.includes("aarch64") || lower.includes("arm64")) return "darwin-aarch64";
  if (lower.includes("x86_64") || /_x64[^0-9]/.test(lower) || lower.endsWith("_x64.dmg")) {
    return "darwin-x86_64";
  }
  return "darwin-aarch64";
}

function partialDarwin(profile) {
  const conf = readConf();
  const version = conf.version;
  if (!version) throw new Error("tauri.conf.json missing version");

  const { owner, repo } = parseGithubRepo(conf);
  const tag = releaseTag(version);
  const bundleRoot = path.join(ROOT, "src-tauri", "target", profile, "bundle");

  const candidates = [];
  for (const sub of ["dmg", "macos"]) {
    const dir = path.join(bundleRoot, sub);
    if (!fs.existsSync(dir)) continue;
    for (const name of fs.readdirSync(dir)) {
      if (name.endsWith(".dmg") || name.endsWith(".tar.gz") || name.endsWith(".zip")) {
        if (name.endsWith(".sig")) continue;
        candidates.push(path.join(dir, name));
      }
    }
  }

  if (candidates.length === 0) {
    throw new Error(`No .dmg / .tar.gz updater bundle under ${bundleRoot}/{dmg,macos}`);
  }

  /** @type {Record<string, { signature: string; url: string }>} */
  const platforms = {};

  for (const filePath of candidates) {
    const name = path.basename(filePath);
    const key = darwinPlatformKey(name);
    if (platforms[key]) {
      throw new Error(`Duplicate Darwin platform ${key} for ${name}; only one bundle per arch supported.`);
    }
    platforms[key] = {
      signature: readSig(filePath),
      url: assetUrl(owner, repo, tag, name),
    };
  }

  const out = { platforms };
  fs.writeFileSync(path.join(ROOT, "platforms.partial.json"), JSON.stringify(out, null, 2) + "\n", "utf8");
  console.log("Wrote platforms.partial.json (darwin)", Object.keys(platforms).join(", "));
}

function mergePartial(pathA, pathB, outPath) {
  const conf = readConf();
  const version = conf.version;
  if (!version) throw new Error("tauri.conf.json missing version");

  const a = readJson(pathA);
  const b = readJson(pathB);
  if (!a.platforms || !b.platforms) {
    throw new Error("Each partial must have a top-level 'platforms' object");
  }

  const platforms = { ...a.platforms, ...b.platforms };
  const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

  const manifest = {
    version,
    notes: process.env.RELEASE_NOTES || "",
    pub_date: pubDate,
    platforms,
  };

  fs.writeFileSync(outPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
  console.log("Wrote", outPath);
}

function usage() {
  console.error(`Usage:
  node scripts/updater-manifest.mjs partial-windows [release|debug]
  node scripts/updater-manifest.mjs partial-darwin [release|debug]
  node scripts/updater-manifest.mjs merge <win.json> <darwin.json> -o <out.json>`);
  process.exit(1);
}

const cmd = process.argv[2];
const arg = process.argv[3] || "release";
const profile = arg === "debug" ? "debug" : "release";

if (cmd === "partial-windows") {
  partialWindows(profile);
} else if (cmd === "partial-darwin") {
  partialDarwin(profile);
} else if (cmd === "merge") {
  const winPath = process.argv[3];
  const darwinPath = process.argv[4];
  let oIdx = process.argv.indexOf("-o");
  if (oIdx === -1 || !process.argv[oIdx + 1]) usage();
  const outPath = process.argv[oIdx + 1];
  if (!winPath || !darwinPath) usage();
  mergePartial(winPath, darwinPath, outPath);
} else {
  usage();
}
