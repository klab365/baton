type Usage = {
	inputTokens?: number | null;
	outputTokens?: number | null;
	cacheReadTokens?: number | null;
	cacheWriteTokens?: number | null;
	estimatedCost?: string | null;
	priceSource?: string | null;
	priceCheckedAt?: string | null;
};
type RunStep = {
	stepId: string;
	attempt: number;
	status: string;
	durationMs?: number | null;
	usage?: Usage | null;
};
type Coverage = "complete" | "partial" | "unavailable";

export function runInsights(allSteps: RunStep[]) {
	const steps = allSteps.filter(
		(step) => step.status !== "pending" || step.attempt > 0,
	);
	const coverage = (count: number): Coverage =>
		count === 0
			? "unavailable"
			: count === steps.length
				? "complete"
				: "partial";
	const metric = (key: keyof Usage) => {
		const values = steps
			.map((step) => step.usage?.[key])
			.filter((value): value is number => typeof value === "number");
		return {
			value: values.length
				? values.reduce((sum, value) => sum + value, 0)
				: null,
			coverage: coverage(values.length),
		};
	};
	const durations = steps
		.map((step) => step.durationMs)
		.filter((value): value is number => typeof value === "number");
	const validCost = (value: string | null | undefined) =>
		value?.trim() !== "" && value != null && Number.isFinite(Number(value));
	const costObservations = steps.flatMap((step) => {
		const usage = step.usage;
		const value = usage?.estimatedCost;
		if (!usage || typeof value !== "string" || !validCost(value)) return [];
		return [
			{
				value: value.trim(),
				priceSource: usage.priceSource,
				priceCheckedAt: usage.priceCheckedAt,
			},
		];
	});
	const input = metric("inputTokens");
	const output = metric("outputTokens");
	const cacheRead = metric("cacheReadTokens");
	const cacheWrite = metric("cacheWriteTokens");
	return {
		attempts: steps.length,
		retries: steps.reduce(
			(count, step, index) =>
				count +
				(steps.slice(0, index).some((prior) => prior.stepId === step.stepId)
					? 1
					: 0),
			0,
		),
		durationMs: durations.length
			? durations.reduce((sum, value) => sum + value, 0)
			: null,
		durationCoverage: coverage(durations.length),
		inputTokens: input.value,
		inputTokenCoverage: input.coverage,
		outputTokens: output.value,
		outputTokenCoverage: output.coverage,
		cacheReadTokens: cacheRead.value,
		cacheReadTokenCoverage: cacheRead.coverage,
		cacheWriteTokens: cacheWrite.value,
		cacheWriteTokenCoverage: cacheWrite.coverage,
		cost: "unavailable",
		costCoverage: coverage(costObservations.length),
		costObservations,
		priceSources: [
			...new Set(
				costObservations
					.map((usage) => usage.priceSource)
					.filter((value): value is string => Boolean(value)),
			),
		],
		priceCheckedAt: [
			...new Set(
				costObservations
					.map((usage) => usage.priceCheckedAt)
					.filter((value): value is string => Boolean(value)),
			),
		],
	};
}
