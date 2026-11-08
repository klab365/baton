import { describe, expect, test } from "bun:test";
import { runFailure } from "./run-failure";

describe("runFailure", () => {
	test("shows setup errors without attempts", () => {
		expect(
			runFailure({ status: "failed", setupError: "worktree error", steps: [] }),
		).toEqual({ title: "Run setup failed", detail: "worktree error" });
	});
	test("uses the latest failed attempt rather than an earlier blocker", () => {
		expect(
			runFailure({
				status: "failed",
				steps: [
					{
						attempt: 3,
						stepId: "build",
						status: "failed",
						error: "provider error",
					},
					{
						attempt: 1,
						stepId: "plan",
						status: "succeeded",
						outcome: "blocked",
					},
				],
			}),
		).toEqual({
			title: "Step build failed",
			detail: "provider error",
			attempt: 3,
		});
	});
	test("explains a terminal rejection even when the step succeeded", () => {
		expect(
			runFailure({
				status: "failed",
				steps: [
					{
						attempt: 2,
						stepId: "approval",
						status: "succeeded",
						outcome: "rejected",
						output: "Not ready",
						selectedTransition: { to: null },
					},
				],
			}),
		).toEqual({
			title: "Run ended: rejected",
			detail: "Not ready",
			attempt: 2,
		});
	});
	test("explains interruption and does not flag successful runs", () => {
		expect(
			runFailure({
				status: "failed",
				steps: [],
				events: [{ message: "Run interrupted by server restart" }],
			})?.title,
		).toBe("Run interrupted");
		expect(runFailure({ status: "succeeded", steps: [] })).toBeNull();
	});
});
