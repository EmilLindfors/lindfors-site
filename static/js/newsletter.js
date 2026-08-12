// Newsletter subscribe forms: POST the address as JSON and report status on the button.
//
// The endpoint comes from each form's own `action`, so this file carries no
// configuration and is safe to load on pages that have no form at all.
// Externalised from base.html for CSP; see theme.js.
(function () {
    document.querySelectorAll('.newsletter-form').forEach(function (form) {
        form.addEventListener('submit', function (e) {
            e.preventDefault();
            var email = form.querySelector('input[name="email"]').value;
            var btn = form.querySelector('button[type="submit"]');
            var originalText = btn.textContent;
            btn.textContent = 'Sending...';
            btn.disabled = true;

            fetch(form.action, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email: email })
            }).then(function (res) {
                if (res.ok) {
                    btn.textContent = 'Subscribed!';
                    form.querySelector('input[name="email"]').value = '';
                    setTimeout(function () {
                        btn.textContent = originalText;
                        btn.disabled = false;
                    }, 3000);
                } else {
                    throw new Error('Failed');
                }
            }).catch(function () {
                btn.textContent = 'Error - try again';
                btn.disabled = false;
                setTimeout(function () { btn.textContent = originalText; }, 3000);
            });
        });
    });
})();
