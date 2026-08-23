// Blog post chrome: TOC scroll spy, reading progress, and the citation modal.
//
// Externalised from page.html for CSP (see theme.js). The `onclick="..."` attributes
// that used to drive the citation modal went with it: inline handlers are blocked by
// any script-src that lacks 'unsafe-inline', and 'unsafe-hashes' would defeat the
// point. Buttons now declare intent with data-* attributes and are wired up here.
(function () {
    document.addEventListener('DOMContentLoaded', function () {
        // Citations quote an access date, which has to be today rather than build day.
        const today = new Date().toLocaleDateString('en-US', {
            year: 'numeric', month: 'long', day: 'numeric'
        });
        document.querySelectorAll('.cite-date').forEach(function (el) {
            el.textContent = today;
        });

        initCiteModal();
        initTocAndProgress();
        initAudioSpeed();
    });

    function initTocAndProgress() {
        const tocLinks = document.querySelectorAll('.toc-nav a');
        const progressBar = document.getElementById('reading-progress');

        const headings = [];
        tocLinks.forEach(function (link) {
            const href = link.getAttribute('href');
            const hashIndex = href.indexOf('#');
            if (hashIndex === -1) return;
            const heading = document.getElementById(href.slice(hashIndex + 1));
            if (heading) headings.push({ heading: heading, link: link });
        });

        function updateActiveLink() {
            if (headings.length === 0) return;
            const scrollPos = window.scrollY + 100;
            let current = headings[0];
            for (const item of headings) {
                if (item.heading.offsetTop <= scrollPos) current = item;
            }
            tocLinks.forEach(function (link) { link.classList.remove('active'); });
            if (current) current.link.classList.add('active');
        }

        function updateProgress() {
            if (!progressBar) return;
            const docHeight = document.documentElement.scrollHeight - window.innerHeight;
            if (docHeight > 0) {
                progressBar.style.width = (window.scrollY / docHeight * 100) + '%';
            }
        }

        // A post with no TOC still has a progress bar, so this is not gated on headings.
        if (headings.length === 0 && !progressBar) return;

        window.addEventListener('scroll', function () {
            updateActiveLink();
            updateProgress();
        }, { passive: true });

        updateActiveLink();
        updateProgress();
    }

    // Playback speed for the audio version. The player itself is native <audio
    // controls>, so everything except these buttons works with JavaScript disabled.
    // The choice is remembered: someone who listens at 1.5x wants that on every post.
    function initAudioSpeed() {
        const player = document.querySelector('.audio-player');
        const buttons = document.querySelectorAll('.audio-rate');
        if (!player || buttons.length === 0) return;

        function apply(rate) {
            player.playbackRate = rate;
            // load() resets playbackRate to defaultPlaybackRate, and preload="none"
            // means the resource is fetched long after this runs. Setting both keeps
            // a restored 1.5x from silently reverting on the first play.
            player.defaultPlaybackRate = rate;
            buttons.forEach(function (button) {
                const active = Number(button.dataset.rate) === rate;
                button.classList.toggle('is-active', active);
                button.setAttribute('aria-pressed', String(active));
            });
        }

        let stored = null;
        try {
            stored = Number(localStorage.getItem('audio-rate'));
        } catch (e) {
            // Private mode, or storage blocked. The default rate is fine.
        }

        const rates = Array.from(buttons, function (b) { return Number(b.dataset.rate); });
        if (stored && rates.indexOf(stored) !== -1) {
            apply(stored);
        }

        buttons.forEach(function (button) {
            button.addEventListener('click', function () {
                const rate = Number(button.dataset.rate);
                apply(rate);
                try {
                    localStorage.setItem('audio-rate', String(rate));
                } catch (e) {
                    // Not worth surfacing: the rate still applies to this page.
                }
            });
        });
    }

    function initCiteModal() {
        const modal = document.getElementById('cite-modal');
        let modalTrigger = null;

        function toggleModal() {
            if (!modal) return;
            const isHidden = modal.style.display === 'none';
            modal.style.display = isHidden ? 'block' : 'none';
            if (isHidden) {
                modalTrigger = document.activeElement;
                const firstTab = modal.querySelector('.cite-tab');
                if (firstTab) firstTab.focus();
            } else if (modalTrigger) {
                modalTrigger.focus();
                modalTrigger = null;
            }
        }

        document.querySelectorAll('[data-cite-toggle]').forEach(function (btn) {
            btn.addEventListener('click', toggleModal);
        });

        document.addEventListener('keydown', function (e) {
            if (e.key === 'Escape' && modal && modal.style.display !== 'none') {
                toggleModal();
            }
        });

        // Format tabs. The old version read the global `event` to find the clicked tab,
        // which is non-standard; the listener's own target is the honest source.
        document.querySelectorAll('[data-cite-format]').forEach(function (tab) {
            tab.addEventListener('click', function () {
                const format = tab.getAttribute('data-cite-format');
                const target = document.getElementById('cite-' + format);
                if (!target) return;
                document.querySelectorAll('.cite-content').forEach(function (el) {
                    el.style.display = 'none';
                });
                document.querySelectorAll('.cite-tab').forEach(function (el) {
                    el.classList.remove('active');
                });
                target.style.display = 'block';
                tab.classList.add('active');
            });
        });

        document.querySelectorAll('[data-cite-copy]').forEach(function (btn) {
            btn.addEventListener('click', function () {
                const format = btn.getAttribute('data-cite-copy');
                const code = document.querySelector('#cite-' + format + ' pre code');
                if (!code) return;
                copyText(code.textContent, btn, 'Copied!');
            });
        });

        document.querySelectorAll('[data-copy-link]').forEach(function (btn) {
            btn.addEventListener('click', function () {
                const label = btn.querySelector('.share-text') || btn;
                copyText(window.location.href, label, 'Copied!');
            });
        });
    }

    /// Copy to the clipboard and flash confirmation on `el`, restoring its label after.
    function copyText(text, el, confirmation) {
        navigator.clipboard.writeText(text).then(function () {
            const original = el.textContent;
            el.textContent = confirmation;
            setTimeout(function () { el.textContent = original; }, 2000);
        });
    }
})();
