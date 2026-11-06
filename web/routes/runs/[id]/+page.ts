import { error } from "@sveltejs/kit";
import type { PageLoad } from "./$types";
export const load: PageLoad = async ({ fetch, params }) => {
	const response = await fetch(`/api/runs/${encodeURIComponent(params.id)}`);
	if (!response.ok) error(response.status, "Could not load run.");
	const run = await response.json();
	let task: { id: string; title: string } | null = null;
	if (run.taskId) {
		const taskResponse = await fetch(
			`/api/tasks/${encodeURIComponent(run.taskId)}`,
		);
		if (taskResponse.ok) task = await taskResponse.json();
	}
	return { run, task };
};
