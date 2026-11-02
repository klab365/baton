import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		adapter: adapter({ fallback: "index.html" }),
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
