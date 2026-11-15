import { expect, test } from "bun:test";
import { runInsights } from "./run-insights";

test("keeps run cost unavailable while retaining individual zero cost and provenance", () => {
	const result = runInsights([
		{
			stepId: "build",
			attempt: 1,
			status: "succeeded",
			durationMs: 0,
			usage: {
				inputTokens: 0,
				outputTokens: 5,
				cacheReadTokens: 2,
				estimatedCost: "0",
				priceSource: "catalog",
				priceCheckedAt: "now",
			},
		},
		{
			stepId: "review",
			attempt: 2,
			status: "succeeded",
			durationMs: null,
			usage: {
				inputTokens: 3,
				outputTokens: null,
				cacheReadTokens: null,
				estimatedCost: "0.02",
				priceSource: "other",
			},
		},
	]);
	expect(result.cost).toBe("unavailable");
	expect(result.costCoverage).toBe("complete");
	expect(result.costObservations.map((cost) => cost.value)).toEqual([
		"0",
		"0.02",
	]);
	expect(result.priceSources).toEqual(["catalog", "other"]);
	expect(result.durationMs).toBe(0);
	expect(result.durationCoverage).toBe("partial");
	expect(result.inputTokenCoverage).toBe("complete");
	expect(result.outputTokenCoverage).toBe("partial");
	expect(result.cacheReadTokens).toBe(2);
});

test("does not mistake multiple ordinary steps' global numbers for retries; filters pending", () => {
	const result = runInsights([
		{ stepId: "build", attempt: 1, status: "succeeded" },
		{ stepId: "review", attempt: 2, status: "running" },
		{ stepId: "finish", attempt: 0, status: "pending" },
	]);
	expect(result.attempts).toBe(2);
	expect(result.retries).toBe(0);
});

test("counts a repeated step visit as a retry", () => {
	const result = runInsights([
		{ stepId: "build", attempt: 1, status: "succeeded" },
		{ stepId: "review", attempt: 2, status: "succeeded" },
		{ stepId: "build", attempt: 3, status: "running" },
	]);
	expect(result.retries).toBe(1);
});

test("keeps missing, empty and invalid cost strings unavailable", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: { estimatedCost: "" },
		},
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: { estimatedCost: "NaN" },
		},
	]);
	expect(result.cost).toBe("unavailable");
	expect(result.costObservations).toEqual([]);
});
