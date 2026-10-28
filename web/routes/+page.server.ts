import { fail, redirect } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";

const apiUrl = process.env.BATON_API_URL ?? "http://127.0.0.1:5173";

async function loadJson<T>(fetcher: typeof fetch, path: string): Promise<T> {
	const response = await fetcher(`${apiUrl}${path}`);
	if (!response.ok) throw new Error(`Could not load ${path}.`);
	return response.json();
}

export const load: PageServerLoad = async ({ fetch }) => {
	const [tasks, runs, credentials] = await Promise.all([
		loadJson<unknown[]>(fetch, "/api/tasks"),
		loadJson<unknown[]>(fetch, "/api/runs"),
		loadJson<unknown[]>(fetch, "/api/credentials"),
	]);
	return { tasks, runs, credentials };
};

export const actions: Actions = {
	initialize: async ({ fetch }) => {
		const response = await fetch(`${apiUrl}/api/project/initialize`, {
			method: "POST",
		});
		if (response.status === 409)
			return fail(409, { message: "A project configuration already exists." });
		if (!response.ok) return fail(500, { message: "Project setup failed." });
		throw redirect(303, "/");
	},
};
