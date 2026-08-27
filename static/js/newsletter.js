// Newsletter subscribe forms: POST the address as JSON and report status below the form.
//
// The endpoint comes from each form's own `action`, so this file carries no
// configuration and is safe to load on pages that have no form at all.
// Externalised from base.html for CSP; see theme.js.
//
// The status message is a persistent element rather than a few seconds of changed
// button text, because since double opt-in the success case is an *instruction*:
// nothing has happened yet, and the reader has to go and click a link in their mail.
// A message that erases itself after three seconds loses exactly the people it was
// written for.
(function () {
    var MESSAGES = {
        pending: 'Almost there — check your inbox and click the confirmation link. It works for 48 hours.',
        invalid: 'That address does not look right. Check it and try again.',
        throttled: 'Too many attempts just now. Wait a minute and try again.',
        failed: 'Something went wrong on our end. Please try again in a moment.'
    };

    document.querySelectorAll('.newsletter-form').forEach(function (form) {
        // One status element per form, created on first use and reused after that so
        // repeated submits do not stack up messages.
        var status = null;

        function say(text, kind) {
            if (!status) {
                status = document.createElement('p');
                status.className = 'newsletter-msg';
                // Announced to screen readers, which otherwise get no signal that
                // anything happened: the message appears well away from the focused
                // button and nothing moves focus to it.
                status.setAttribute('role', 'status');
                form.parentNode.insertBefore(status, form.nextSibling);
            }
            status.className = 'newsletter-msg newsletter-msg--' + kind;
            status.textContent = text;
        }

        form.addEventListener('submit', function (e) {
            e.preventDefault();
            var input = form.querySelector('input[name="email"]');
            var btn = form.querySelector('button[type="submit"]');
            var originalText = btn.textContent;

            btn.textContent = 'Sending...';
            btn.disabled = true;

            fetch(form.action, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email: input.value })
            }).then(function (res) {
                // The body is JSON on every path, including the errors, so read it
                // before branching -- the 400 carries the reason the address was
                // rejected and it is more useful than anything invented here.
                return res.json().catch(function () { return {}; }).then(function (data) {
                    return { status: res.status, data: data };
                });
            }).then(function (r) {
                if (r.status === 200) {
                    say(MESSAGES.pending, 'ok');
                    input.value = '';
                } else if (r.status === 400) {
                    say(r.data.error || MESSAGES.invalid, 'err');
                } else if (r.status === 429) {
                    say(MESSAGES.throttled, 'err');
                } else {
                    say(MESSAGES.failed, 'err');
                }
            }).catch(function () {
                say(MESSAGES.failed, 'err');
            }).finally(function () {
                // The form stays usable either way: on success so a second address can
                // be added, on failure so a typo can be corrected without a reload.
                btn.textContent = originalText;
                btn.disabled = false;
            });
        });
    });
})();
