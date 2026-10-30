/** The copy button beside a command (Command in content/schema.flecs). */
for (const button of document.querySelectorAll<HTMLButtonElement>('button[data-copy]')) {
	button.addEventListener('click', async () => {
		await navigator.clipboard.writeText(button.dataset.copy ?? '');
		button.textContent = 'Copied';
		setTimeout(() => (button.textContent = 'Copy'), 1500);
	});
}
