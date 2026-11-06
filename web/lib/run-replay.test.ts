import { describe, expect, test } from "bun:test";
import { replaySteps } from "./run-replay";

describe("replaySteps", () => {
	test("orders repeated steps by attempt and excludes unvisited steps", () => {
		const steps = [
			{ stepId: "review", attempt: 3, status: "succeeded" },
			{ stepId: "review", attempt: 1, status: "succeeded" },
			{ stepId: "human", attempt: 2, status: "succeeded" },
			{ stepId: "unused", attempt: 0, status: "pending" },
		];
		expect(replaySteps(steps).map((step) => step.attempt)).toEqual([1, 2, 3]);
		expect(steps[0].attempt).toBe(3);
	});

	test("preserves saved order when older attempts lack numbers", () => {
		const steps = [
			{ attempt: 0, status: "succeeded" },
			{ attempt: 2, status: "failed" },
		];
		expect(replaySteps(steps)).toEqual(steps);
	});
});
