type Attempt = {
	attempt: number;
	stepId: string;
	status: string;
	error?: string | null;
	outcome?: string | null;
	output?: string | null;
	selectedTransition?: { to?: string | null } | null;
};

type FailedRun = {
	status: string;
	setupError?: string | null;
	steps: Attempt[];
	events?: { message?: string | null }[];
};

export function runFailure(
	run: FailedRun,
): { title: string; detail: string; attempt?: number } | null {
	if (run.status !== "failed" && run.status !== "limitExhausted") return null;
	if (run.setupError)
		return { title: "Run setup failed", detail: run.setupError };
	const latest = run.steps
		.filter((step) => step.attempt > 0)
		.sort((a, b) => b.attempt - a.attempt)[0];
	if (latest?.error)
		return {
			title: `Step ${latest.stepId} failed`,
			detail: latest.error,
			attempt: latest.attempt,
		};
	if (
		latest?.selectedTransition &&
		latest.selectedTransition.to == null &&
		["blocked", "failed", "rejected"].includes(latest.outcome ?? "")
	) {
		return {
			title: `Run ended: ${latest.outcome}`,
			detail:
				latest.output?.trim() || "No additional explanation was provided.",
			attempt: latest.attempt,
		};
	}
	const interruption = [...(run.events ?? [])]
		.reverse()
		.find((event) => event.message?.includes("interrupted by server restart"));
	if (interruption?.message)
		return { title: "Run interrupted", detail: interruption.message };
	return {
		title: run.status === "limitExhausted" ? "Run limit reached" : "Run failed",
		detail:
			"No detailed cause is available. Review the timeline and run events.",
		...(latest ? { attempt: latest.attempt } : {}),
	};
}
