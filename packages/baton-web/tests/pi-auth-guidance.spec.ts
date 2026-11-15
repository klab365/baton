import { expect as baseExpect, test } from "@playwright/test";

const expect = baseExpect.configure({ timeout: 15000 });

test("Pi settings keeps auth source and unknown status visible and scopes instructions to the selected provider", async ({
	page,
}) => {
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/credentials") await route.fulfill({ json: [] });
		else if (path === "/api/credential-types")
			await route.fulfill({ json: [] });
		else await route.fulfill({ json: {} });
	});
	await page.goto("/settings");
	await expect(page.getByText(/Authentication status: unknown/)).toBeVisible();
	await expect(
		page.getByText(/Credential source: Pi-managed local credentials/),
	).toBeVisible();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Other / custom provider" }).click();
	await page
		.getByLabel("Custom Pi provider ID")
		.fill("example-custom-provider");
	await expect(
		page.getByText(
			/Baton has no verified provider-specific authentication instructions/,
		),
	).toBeVisible();
	await expect(
		page.getByText(/No method or readiness is inferred here/),
	).toBeVisible();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Anthropic" }).click();
	await expect(page.getByText(/ANTHROPIC_API_KEY/)).toBeVisible();
	await expect(
		page.getByText(/pi auth login anthropic --api-key/),
	).toBeVisible();
	await expect(page.getByText(/workload identity federation/)).toBeVisible();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Amazon Bedrock" }).click();
	await expect(page.getByText(/ambient cloud credentials/)).toBeVisible();
});
