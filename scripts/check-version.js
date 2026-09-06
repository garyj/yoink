import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const pkg = JSON.parse(readFileSync("package.json", "utf8"));
const tauri = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], { encoding: "utf8" }));
if (!/^\d+\.\d+\.\d+$/.test(pkg.version)) throw new Error("Use a stable major.minor.patch version");
if (tauri.version !== pkg.version || metadata.packages.some((/** @type {{version: string}} */ crate) => crate.version !== pkg.version)) {
  throw new Error("package.json, tauri.conf.json, and both Cargo packages must have the same version");
}
if (process.env.GITHUB_REF_TYPE === "tag" && process.env.GITHUB_REF_NAME !== `v${pkg.version}`) {
  throw new Error("Release tag must match the package version");
}
console.log(pkg.version);
