import type { PageLoad } from "./$types";

type Run = {
	id: string;
	taskId?: string | null;
	pipelineId: string;
	status: string;
	startedAt: string;
	endedAt?: string | null;
	currentStep?: string | null;
};
type Task = { id: string; title: string };

export const load: PageLoad = async ({ fetch }) => {
	const [runsResponse, tasksResponse] = await Promise.all([
		fetch("/api/runs"),
		fetch("/api/tasks"),
	]);
	if (!runsResponse.ok || !tasksResponse.ok)
		throw new Error("Could not load runs.");
	const [runs, tasks] = await Promise.all([
		runsResponse.json() as Promise<Run[]>,
		tasksResponse.json() as Promise<Task[]>,
	]);
	return { runs, tasks };
};
