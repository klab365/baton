import tailwindcss from "@tailwindcss/vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		proxy: { "/api": process.env.BATON_API_URL ?? "http://127.0.0.1:5173" },
	},
	optimizeDeps: {
		exclude: ["bits-ui", "@lucide/svelte"],
	},
});
