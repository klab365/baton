import { fail, redirect } from "@sveltejs/kit";
import type { Actions } from "./$types";

const apiUrl = process.env.BATON_API_URL ?? "http://127.0.0.1:5173";

export const actions: Actions = {
	initialize: async ({ fetch }) => {
		const response = await fetch(`${apiUrl}/api/project/initialize`, {
			method: "POST",
		});
		if (response.status === 409)
			return fail(409, { message: "A project configuration already exists." });
		if (!response.ok) return fail(500, { message: "Project setup failed." });
		throw redirect(303, "/");
	},
};
