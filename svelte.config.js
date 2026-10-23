import adapter from "@sveltejs/adapter-node";

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		adapter: adapter(),
		alias: {
			$components: "web/components",
		},
		files: {
			lib: "web/lib",
			routes: "web/routes",
			appTemplate: "web/app.html",
		},
	},
};

export default config;
