import { expect, test } from "bun:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import baton from "./baton";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { choosePipeline, drive, manage } from "./baton";

function ui(
	overrides: Partial<Parameters<typeof drive>[2]> = {},
): Parameters<typeof drive>[2] {
	return {
		select: async () => undefined,
		editor: async () => undefined,
		confirm: async () => false,
		notify: () => {},
		setStatus: () => {},
		...overrides,
	};
}

test("selects newest attempts by global attempt number, not stored placeholder order", async () => {
	const messages: string[] = [];
	const steps = Array.from({ length: 40 }, (_, index) => ({
		stepId: `step-${index}`, attempt: 40 - index, status: "succeeded",
	}));
	steps[0] = { stepId: "latest", attempt: 41, status: "running" };
	steps[1] = { stepId: "pending-placeholder", attempt: 0, status: "pending" };
	await manage("inspect run-order", async () => ({ id: "run-order", status: "running", steps }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("latest");
	expect(messages[0]).toContain('"stepId": "step-10"');
	expect(messages[0]).not.toContain('"stepId": "step-39"');
	expect(messages[0]).not.toContain("pending-placeholder");
});

test("bounds large cost summaries and preserves run identity and attempt details", async () => {
	const messages: string[] = [];
	const steps = Array.from({ length: 40 }, (_, attempt) => ({
		stepId: `step-${attempt}`, attempt: attempt + 1, status: "succeeded", durationMs: 1,
		usage: { estimatedCost: "0.01", priceSource: "x".repeat(1000), inputTokens: 1 },
	}));
	await manage("inspect run-long", async () => ({ id: "run-long", status: "succeeded", steps }), ui(), (text) => messages.push(text));
	expect(messages[0].length).toBeLessThanOrEqual(8000);
	expect(messages[0]).toContain("run-long");
	expect(messages[0]).toContain("step-39");
	expect(messages[0]).toContain("omittedEarlierSteps");
});

test("Pi inspection summarizes persisted metrics without treating missing values as zero", async () => {
	const messages: string[] = [];
	await manage("inspect run-1", async () => ({ id: "run-1", steps: [
		{ stepId: "build", attempt: 1, status: "succeeded", durationMs: 0, usage: { inputTokens: 0, outputTokens: 4, cacheReadTokens: 2, estimatedCost: "0.01", priceSource: "catalog", priceCheckedAt: "now" } },
		{ stepId: "build", attempt: 2, status: "failed", durationMs: null, selectedTransition: { to: "build" }, usage: { inputTokens: 1 } },
		{ stepId: "finish", attempt: 0, status: "pending" },
	] }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Attempts: 2 · retries: 1");
	expect(messages[0]).toContain("1 (complete) / 4 (partial) / 2 (partial) / unavailable (unavailable)");
	expect(messages[0]).toContain("Run cost: unavailable (currency/unit not recorded)");
	expect(messages[0]).toContain("Attempt costs: 0.01 · catalog · checked now");
	expect(messages[0]).toContain("Tokens input/output/cache read/cache write: 1 (complete) / 4 (partial) / 2 (partial) / unavailable (unavailable)");
	expect(messages[0]).toContain("Duration: 0 ms (partial)");
});

test("shipped single-file Pi extension has no project-relative runtime imports", async () => {
	const source = await readFile(resolve(import.meta.dir, "baton.ts"), "utf8");
	expect(source).not.toMatch(/from [\"']\.\.?\//);
});

test("lists and inspects persisted work sessions without executing them", async () => {
	const calls: string[][] = [];
	const messages: string[] = [];
	const exec = async (args: string[]) => {
		calls.push(args);
		return args.length === 1 ? [{ id: "run-1", status: "running" }] : { id: "run-1", steps: [] };
	};
	expect(await manage("list", exec, ui(), (text) => messages.push(text))).toBe(true);
	expect(await manage("inspect run-1", exec, ui(), (text) => messages.push(text))).toBe(true);
	expect(calls).toEqual([["status"], ["status", "--run", "run-1"]]);
	expect(messages.join("\\n")).toContain("Work sessions");
	expect(messages.join("\\n")).toContain("run-1");
});

test("paginates session lists so records beyond the first page remain accessible", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 55 }, (_, i) => ({ id: `run-${i}`, status: "succeeded" }));
	await manage("list 2", async () => runs, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("run-50");
	expect(messages[0]).toContain("Page 2 of 2");
});

test("keeps long session records reachable across bounded list pages", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 60 }, (_, i) => ({ id: `run-${i}`, status: "succeeded", currentStep: `step-${i}-${"x".repeat(500)}` }));
	for (let page = 1; page <= 10; page++) {
		await manage(`list ${page}`, async () => runs, ui(), (text) => messages.push(text));
		const pageCount = Number(messages.at(-1)?.match(/Page \d+ of (\d+)/)?.[1]);
		if (page >= pageCount) break;
	}
	const text = messages.join("\\n");
	expect(text.length).toBeLessThan(8500 * 10);
	expect(Array.from(text.matchAll(/"id": "(run-\d+)"/g), (match) => match[1]).sort()).toEqual(runs.map((run) => run.id).sort());
	expect(text).toContain("currentStep truncated");
});

test("marks omitted work sessions and inspection data when output is bounded", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 55 }, (_, i) => ({ id: `run-${i}`, status: "succeeded" }));
	await manage("list", async () => runs, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Page 1 of 2");
	expect(messages[0]).toContain("next: /baton list 2");
	messages.length = 0;
	await manage("inspect run-1", async () => ({
		id: "run-1", steps: Array.from({ length: 32 }, (_, i) => ({ stepId: `step-${i}`, output: "x".repeat(1200) })),
	}), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Work session run-1");
	expect(messages[0]).toContain("2 earlier steps omitted");
	expect(messages[0]).toContain("output truncated");
});

test("resumes a selected work session through the existing driver", async () => {
	const calls: string[][] = [];
	await manage("resume run-1", async (args) => {
		calls.push(args);
		return { status: "succeeded" };
	}, ui(), () => {});
	expect(calls).toEqual([["next", "--run", "run-1"]]);
});

test("refines a completed session through a new run in the same worktree", async () => {
	const calls: string[][] = [];
	const messages: string[] = [];
	const result = await manage(
		"refine",
		async (args) => {
			calls.push(args);
			if (args.length === 1) return [
				{ id: "older-run", status: "succeeded", pipelineId: "software-factory", startedAt: "2026-01-01T00:00:00Z" },
				{ id: "latest-run", status: "succeeded", pipelineId: "software-factory", startedAt: "2026-02-01T00:00:00Z" },
			];
			if (args[0] === "refine") return { id: "new-run", status: "running" };
			return { id: "new-run", status: "succeeded" };
		},
		ui({
			select: async (_prompt, options) => {
				expect(options[0]).toContain("latest-run");
				expect(options[0]).toContain("★");
				return options[0];
			},
			editor: async () => "Fix the remaining edge case",
		}),
		(text) => messages.push(text),
	);
	expect(result).toBe(true);
	expect(calls).toEqual([
		["status"],
		["refine", "--run", "latest-run", "--instruction", "Fix the remaining edge case"],
		["next", "--run", "new-run"],
	]);
	expect(messages.join("\n")).toContain("Baton refinement started: new-run (from latest-run)");
});

test("documents command side effects in help and rejects malformed commands", async () => {
	const messages: string[] = [];
	expect(await manage("help", async () => [], ui(), (text) => messages.push(text))).toBe(true);
	expect(messages[0]).toContain("list [PAGE]");
	expect(messages[0]).toContain("ask for human decisions");
	await expect(manage("inspect", async () => [], ui(), () => {})).rejects.toThrow("Usage:");
});

test("cancelling the run picker or refinement editor never starts a run", async () => {
	const pickerCalls: string[][] = [];
	await manage("refine", async (args) => {
		pickerCalls.push(args);
		return args.length === 1 ? [{ id: "done", status: "succeeded" }] : { id: "new-run" };
	}, ui({ select: async () => undefined }), () => {});
	expect(pickerCalls).toEqual([["status"]]);

	const editorCalls: string[][] = [];
	await manage("refine done", async (args) => {
		editorCalls.push(args);
		return args.length === 1 ? [{ id: "done", status: "succeeded" }] : { id: "new-run" };
	}, ui({ editor: async () => undefined }), () => {});
	expect(editorCalls).toEqual([["status"]]);
});

test("refine rejects explicitly selected runs that are not completed", async () => {
	const calls: string[][] = [];
	await expect(manage("refine failed", async (args) => {
		calls.push(args);
		return [{ id: "failed", status: "failed" }];
	}, ui(), () => {})).rejects.toThrow("No completed work session with ID 'failed'");
	expect(calls).toEqual([["status"]]);
});

test("registered inspect command routes to work status", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const calls: string[][] = [];
	const output: string[] = [];
	const pi = {
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async (_command: string, args: string[]) => {
			calls.push(args);
			return { code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify({ id: "run-1", status: "running", task: "do not expose", configSnapshot: { secret: "hidden" }, steps: [{ stepId: "build", status: "succeeded", output: "x".repeat(5000) }] }) };
		},
		sendMessage: ({ content }: { content: string }) => { output.push(content); },
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("inspect run-1", {
		hasUI: true, cwd: "/tmp", ui: { setStatus() {}, notify() {} },
	} as any);
	expect(calls).toEqual([["work", "status", "--run", "run-1"]]);
	expect(output.join(" ")).toContain("run-1");
	expect(output.join(" ")).not.toContain("do not expose");
	expect(output.join(" ")).not.toContain("hidden");
	expect(output.join(" ").length).toBeLessThan(8000);
});

test("registered refine command starts the follow-up from the current Pi worktree", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const calls: string[][] = [];
	const pi = {
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async (_command: string, args: string[]) => {
			calls.push(args);
			const value = args[1] === "status"
				? [{ id: "old-run", status: "succeeded" }]
				: args[1] === "refine"
					? { id: "new-run", status: "running" }
					: { id: "new-run", status: "succeeded" };
			return { code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify(value) };
		},
		sendMessage: () => {},
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("refine old-run", {
		hasUI: true,
		cwd: "/tmp/worktree",
		ui: {
			select: async (_prompt: string, options: string[]) => options[0],
			editor: async () => "Check the CI failure and fix it",
			setStatus() {},
			notify() {},
		},
	} as any);
	expect(calls).toEqual([
		["work", "status"],
		["work", "refine", "--run", "old-run", "--instruction", "Check the CI failure and fix it"],
		["work", "next", "--run", "new-run"],
	]);
});

test("chooses a pipeline from the keyboard-selectable list", async () => {
	const calls: string[][] = [];
	const selected = await choosePipeline(
		async (args) => {
			calls.push(args);
			return ["factory", "review"];
		},
		ui({ select: async (_prompt, options) => options[1] }),
	);
	expect(calls).toEqual([["pipelines"]]);
	expect(selected).toBe("review");
});

test("pipeline selection cancellation and empty configuration do not select", async () => {
	const warnings: string[] = [];
	expect(
		await choosePipeline(async () => ["factory"], ui({ select: async () => undefined })),
	).toBeUndefined();
	expect(
		await choosePipeline(
			async () => [],
			ui({ notify: (message) => warnings.push(message) }),
		),
	).toBeUndefined();
	expect(warnings).toEqual(["Baton: no pipelines are configured"]);
});

test("surfaces pipeline listing errors", async () => {
	await expect(choosePipeline(async () => { throw new Error("invalid config"); }, ui())).rejects.toThrow("invalid config");
});

const human = {
	runId: "run",
	revision: 2,
	status: "waitingHuman",
	step: { id: "help", name: "Help", human: true },
	attempt: 3,
	allowedOutcomes: ["approved", "rejected"],
	lastResult: { stepId: "build", attempt: 2, output: "Please approve" },
};

test("runs steps through CLI and shows the result", async () => {
	const commands: string[] = [];
	const messages: string[] = [];
	const statuses: (string | undefined)[] = [];
	let completed = false;
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			if (args[0] === "execute") {
				completed = true;
				return {};
			}
			return completed
				? {
						status: "succeeded",
						lastResult: { stepId: "build", attempt: 1, output: "Done" },
					}
				: {
						status: "running",
						step: { id: "build", name: "Build" },
						attempt: 1,
					};
		},
		ui({ setStatus: (_key, value) => statuses.push(value) }),
		(text) => messages.push(text),
	);
	expect(commands).toEqual(["next", "execute", "next", "next"]);
	expect(messages.join("\n")).toContain("Baton run — RUNNING");
	expect(statuses).toContain("Baton · RUNNING · Build · attempt 1");
	expect(messages.join("\n")).toContain("Attempt: 1");
	expect(messages.join("\n")).toContain("Done");
});

test("formats extension failures as Baton error feedback", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const notices: string[] = [];
	const pi = {
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async () => ({ code: 1, killed: false, truncated: false, stderr: "offline", stdout: "" }),
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("run-1", { hasUI: true, cwd: "/tmp", ui: { notify: (message: string) => notices.push(message), setStatus() {} } } as any);
	expect(notices[0]).toContain("Baton run-1 — ERROR");
	expect(notices[0]).toContain("offline");
});

test("makes a human handoff prominent with decisions and resume instructions", async () => {
	const messages: string[] = [];
	await drive("run-1", async () => human, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Waiting for your decision");
	expect(messages[0]).toContain("approved, rejected");
	expect(messages[0]).toContain("resume run-1");
});

test("pages long human handoffs on demand without flooding the terminal", async () => {
	const messages: string[] = [];
	const choices: string[] = [];
	const output = `${"A".repeat(7000)}${"B".repeat(7000)}${"C".repeat(7000)}`;
	await drive("run-1", async () => ({ ...human, lastResult: { ...human.lastResult!, output } }), ui({
		select: async (prompt, options) => {
			choices.push(prompt);
			return prompt.includes("handoff") ? options[0] : undefined;
		},
	}), (text) => messages.push(text));
	expect(messages).toHaveLength(4);
	expect(messages[0]).toContain("resume run-1");
	expect(messages[0].indexOf("resume run-1")).toBeLessThan(messages[0].indexOf("Prior handoff:"));
	expect(messages.slice(1)).toEqual(["A".repeat(7000), "B".repeat(7000), "C".repeat(7000)]);
	expect(messages.every((message) => message.length <= 8000)).toBe(true);
	expect(choices.filter((prompt) => prompt.includes("handoff"))).toHaveLength(2);
});

test("does not submit a human decision when selection is cancelled", async () => {
	const commands: string[] = [];
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			return human;
		},
		ui(),
		() => {},
	);
	expect(commands).toEqual(["next"]);
});

test("does not submit a decision when confirmation is declined", async () => {
	const commands: string[] = [];
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			return human;
		},
		ui({
			select: async () => "approved",
			editor: async () => "Proceed",
			confirm: async () => false,
		}),
		() => {},
	);
	expect(commands).toEqual(["next"]);
});

test("submits only the explicitly confirmed human decision", async () => {
	let submitted: unknown;
	let waiting = true;
	const messages: string[] = [];
	await drive(
		"run",
		async (args) => {
			if (args[0] === "next") return waiting ? human : { status: "succeeded" };
			if (args[0] === "respond") {
				submitted = JSON.parse(
					await readFile(args[args.indexOf("--result") + 1], "utf8"),
				);
				waiting = false;
				return {};
			}
			throw new Error("unexpected command");
		},
		ui({
			select: async () => "approved",
			editor: async () => "Proceed",
			confirm: async () => true,
		}),
		(text) => messages.push(text),
	);
	expect(submitted).toEqual({
		revision: 2,
		stepId: "help",
		attempt: 3,
		outcome: "approved",
		response: "Proceed",
	});
	expect(messages.join("\n")).toContain("Please approve");
});

test("clears the status when a CLI command fails", async () => {
	const statuses: (string | undefined)[] = [];
	await expect(
		drive(
			"run",
			async () => {
				throw new Error("offline");
			},
			ui({
				setStatus: (_key, value) => {
					statuses.push(value);
				},
			}),
			() => {},
		),
	).rejects.toThrow("offline");
	expect(statuses).toEqual([undefined]);
});
