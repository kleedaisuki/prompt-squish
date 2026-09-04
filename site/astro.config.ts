import { defineConfig } from "astro/config";
import react from "@astrojs/react";

export default defineConfig({
  site: "https://xmlsquish.moesegfault.dev",
  output: "static",
  integrations: [react()],
});
