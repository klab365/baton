import { error } from "@sveltejs/kit";
import type { PageLoad } from "./$types";
export const load: PageLoad = async ({ fetch, params }) => {
	const response = await fetch(`/api/tasks/${encodeURIComponent(params.id)}`);
	if (!response.ok) error(response.status, "Could not load task.");
	return { task: await response.json() };
};
