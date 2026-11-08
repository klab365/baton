export type TaskBoardTask = {
	id: string;
	title: string;
	description: string;
	pipelineId: string;
	status: "draft" | "doing" | "done";
};

export type TaskBoardRun = {
	id: string;
	taskId?: string | null;
	status: string;
	startedAt: string;
	setupError?: string | null;
	steps: { error?: string | null }[];
};
