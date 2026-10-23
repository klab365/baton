import type { PageLoad } from "./$types";
export const load: PageLoad = async ({ fetch }) => {
	const response = await fetch("/api/tasks");
	if (!response.ok) throw new Error("Could not load tasks.");
	return { tasks: await response.json() };
};
