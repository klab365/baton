import type { PageLoad } from "./$types";
import type { TaskBoardRun, TaskBoardTask } from "$lib/task-board";

async function loadJson<T>(url: string, fetcher: typeof fetch): Promise<T> {
	const response = await fetcher(url);
	if (!response.ok) throw new Error(`Could not load ${url}.`);
	return response.json();
}

export const load: PageLoad = ({ fetch }) => ({
	tasks: loadJson<TaskBoardTask[]>("/api/tasks", fetch),
	runs: loadJson<TaskBoardRun[]>("/api/runs", fetch),
});
