type ReplayStep = { attempt: number; status: string };

/** Preserve stored order for legacy steps without attempt numbers. */
export function replaySteps<T extends ReplayStep>(steps: T[]): T[] {
	const recorded = steps.filter(
		(step) => step.attempt > 0 || step.status !== "pending",
	);
	return recorded.every((step) => step.attempt > 0)
		? recorded.sort((a, b) => a.attempt - b.attempt)
		: recorded;
}
