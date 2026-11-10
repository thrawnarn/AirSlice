// Build the card into the integration's www/ folder, which it serves.
import * as esbuild from "esbuild";

await esbuild.build({
  entryPoints: ["src/airslice-history-card.js"],
  outfile: "../custom_components/airslice/www/airslice-history-card.js",
  bundle: true,
  format: "esm",
  target: "es2021",
  // Readable: this file is what a browser's debugger shows.
  minify: false,
  legalComments: "eof",
  banner: { js: "// airslice-history-card -- GENERATED from card/src by `npm run build`. Edit the source and rebuild." },
  logLevel: "info",
});
