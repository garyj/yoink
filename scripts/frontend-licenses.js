import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";

/** @returns {import("vite").Plugin} */
export function frontendLicenses() {
  return {
    name: "frontend-licenses",
    generateBundle(_options, bundle) {
      const packages = new Set();
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk") continue;
        for (const id of Object.keys(chunk.modules)) {
          if (!id.includes("/node_modules/")) continue;
          let directory = dirname(id.split("?")[0]);
          while (!existsSync(join(directory, "package.json"))) {
            const parent = dirname(directory);
            if (parent === directory) throw new Error(`Cannot locate package for ${id}`);
            directory = parent;
          }
          packages.add(directory);
        }
      }
      const sections = [];
      for (const directory of [...packages].sort()) {
        const pkg = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
        const notices = readdirSync(directory).filter((name) => /^(licen[sc]e|copying|notice)([.-]|$)/i.test(name));
        let text = notices.map((name) => readFileSync(join(directory, name), "utf8")).join("\n\n");
        if (pkg.name === "@tauri-apps/api") {
          text = readFileSync(new URL("../licenses/tauri-api-MIT.txt", import.meta.url), "utf8");
          const tslib = readFileSync(join(directory, "external/tslib/tslib.es6.js"), "utf8");
          text += `\n\n${tslib.slice(0, tslib.indexOf("*/") + 2)}`;
        }
        if (!text) throw new Error(`Missing bundled dependency licence: ${pkg.name}`);
        sections.push(`${pkg.name} ${pkg.version}\n${text}`);
      }
      this.emitFile({ type: "asset", fileName: "FRONTEND_LICENSES.txt", source: sections.join("\n\n") });
    },
  };
}
