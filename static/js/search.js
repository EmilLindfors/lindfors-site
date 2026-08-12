// Client-side search over the elasticlunr index Zola generates.
//
// Expects elasticlunr.min.js and search_index.en.js to have loaded first.
// Externalised from search.html for CSP (see theme.js); the form's
// `onsubmit="return false;"` moved here as a preventDefault listener.
(function () {
    var input = document.getElementById('search-input');
    var results = document.getElementById('search-results');
    var form = document.getElementById('search-form');
    var index;

    if (!input || !results) return;

    // Enter must not reload the page: results update as you type.
    if (form) {
        form.addEventListener('submit', function (e) { e.preventDefault(); });
    }

    if (window.searchIndex) {
        index = elasticlunr.Index.load(window.searchIndex);
    }

    function search(query) {
        if (!index || !query || query.length < 2) {
            results.innerHTML = '';
            return;
        }

        var hits = index.search(query, {
            fields: { title: { boost: 2 }, body: { boost: 1 } },
            bool: 'OR',
            expand: true
        });

        if (hits.length === 0) {
            results.innerHTML = '<p class="search-empty">No results found.</p>';
            return;
        }

        var html = '<ul class="post-list">';
        hits.forEach(function (hit) {
            var doc = hit.doc || index.documentStore.getDoc(hit.ref);
            if (!doc) return;

            var body = doc.body || '';
            var snippet = '';
            var lowerBody = body.toLowerCase();
            var lowerQuery = query.toLowerCase();
            var pos = lowerBody.indexOf(lowerQuery);
            if (pos > -1) {
                var start = Math.max(0, pos - 80);
                var end = Math.min(body.length, pos + query.length + 80);
                snippet = (start > 0 ? '...' : '') + body.slice(start, end) + (end < body.length ? '...' : '');
            } else {
                snippet = body.slice(0, 160) + (body.length > 160 ? '...' : '');
            }

            var url = hit.ref;
            var title = doc.title || url;

            html += '<li class="post-item">';
            html += '<a href="' + url + '">';
            html += '<span class="post-title">' + escapeHtml(title) + '</span>';
            html += '</a>';
            html += '<p class="post-excerpt">' + escapeHtml(snippet) + '</p>';
            html += '</li>';
        });
        html += '</ul>';
        results.innerHTML = html;
    }

    function escapeHtml(str) {
        var div = document.createElement('div');
        div.textContent = str;
        return div.innerHTML;
    }

    var timer;
    input.addEventListener('input', function () {
        clearTimeout(timer);
        timer = setTimeout(function () { search(input.value.trim()); }, 200);
    });

    var params = new URLSearchParams(window.location.search);
    var q = params.get('q');
    if (q) {
        input.value = q;
        search(q);
    }
})();
