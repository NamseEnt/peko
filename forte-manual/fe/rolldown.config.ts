import { defineConfig } from "rolldown";

export default defineConfig({
  input: "src/server.js",
  output: {
    file: "dist/server.js",
    inlineDynamicImports: true,
  },
});
