import type { PageLoad } from "./$types";

async function loadJson<T>(fetcher: typeof fetch, path: string): Promise<T> {
	const response = await fetcher(path);
	if (!response.ok) throw new Error(`Could not load ${path}.`);
	return response.json();
}

export const load: PageLoad = async ({ fetch }) => {
	const [tasks, runs, credentials] = await Promise.all([
		loadJson<unknown[]>(fetch, "/api/tasks"),
		loadJson<unknown[]>(fetch, "/api/runs"),
		loadJson<unknown[]>(fetch, "/api/credentials"),
	]);
	return { tasks, runs, credentials };
};
