import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "./tests",
	use: { baseURL: "http://127.0.0.1:5174", browserName: "chromium" },
	webServer: {
		command: "bun run dev -- --host 127.0.0.1 --port 5174",
		url: "http://127.0.0.1:5174",
		reuseExistingServer: !process.env.CI,
	},
});
