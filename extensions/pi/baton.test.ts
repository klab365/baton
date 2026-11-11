import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { drive } from "./baton";

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
		ui(),
		(text) => messages.push(text),
	);
	expect(commands).toEqual(["next", "execute", "next", "next"]);
	expect(messages.join("\n")).toContain("Done");
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
