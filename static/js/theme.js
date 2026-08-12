// Light/dark theme toggle, persisted in localStorage.
//
// Externalised from base.html so the site can ship a Content-Security-Policy with
// script-src 'self' and no 'unsafe-inline'. Cloudflare Pages serves static _headers,
// so there is no per-request nonce available; a separate file is the only option that
// does not degrade to hash churn on every edit.
(function () {
    const toggle = document.getElementById('theme-toggle');
    const html = document.documentElement;

    const savedTheme = localStorage.getItem('theme');
    const systemPrefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;

    if (savedTheme) {
        html.setAttribute('data-theme', savedTheme);
    } else if (systemPrefersDark) {
        html.setAttribute('data-theme', 'dark');
    }

    if (!toggle) return;

    toggle.addEventListener('click', function () {
        const currentTheme = html.getAttribute('data-theme');
        const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
        html.setAttribute('data-theme', newTheme);
        localStorage.setItem('theme', newTheme);
    });
})();
