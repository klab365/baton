import { expect, test } from "@playwright/test";

test("saving an unchanged Rig agent preserves its credential and model", async ({
	page,
}) => {
	let saved:
		| { agent?: { credentialId?: string | null; model?: string | null } }
		| undefined;
	const agent = {
		id: "rig-agent",
		instructions: "test role",
		provider: "rig",
		model: "openai/gpt-4.1",
		credentialId: "rig-key",
		skillIds: [],
		allowedTools: [],
	};
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents" && route.request().method() === "GET")
			await route.fulfill({
				json: { revision: "rev", agents: [agent], skills: [] },
			});
		else if (
			path === "/api/agents/rig-agent" &&
			route.request().method() === "PUT"
		) {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else if (path === "/api/credentials/rig-key/models")
			await route.fulfill({ json: [{ id: "gpt-4.1", label: "GPT-4.1" }] });
		else if (path === "/api/credentials")
			await route.fulfill({
				json: [
					{
						id: "rig-key",
						name: "Rig key",
						credentialType: "openai",
						status: "available",
					},
				],
			});
		else if (path === "/api/credential-types")
			await route.fulfill({
				json: [{ id: "openai", label: "OpenAI", provider: "openai" }],
			});
		else await route.fulfill({ json: {} });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: /rig-agent/ }).click();
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("status")).toContainText("Agent saved");
	await expect.poll(() => saved?.agent?.credentialId).toBe("rig-key");
	await expect.poll(() => saved?.agent?.model).toBe("openai/gpt-4.1");
});
