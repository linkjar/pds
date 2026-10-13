// The live numbers of the overview: every two seconds the page asks the
// console for the state and writes it into the elements that name a value.
(function () {
  var tiles = document.querySelectorAll('[data-live]');
  if (!tiles.length) { return; }
  var shown = document.body.getAttribute('data-subscribers');
  function value(from, path) {
    return path.split('.').reduce(function (at, key) { return at == null ? at : at[key]; }, from);
  }
  function text(given) { return given == null ? '–' : String(given); }
  function tick() {
    fetch('/live.json', { credentials: 'same-origin' })
      .then(function (answer) { return answer.ok ? answer.json() : null; })
      .then(function (live) {
        if (!live) { return; }
        tiles.forEach(function (tile) {
          tile.textContent = text(value(live.overview, tile.getAttribute('data-live')));
        });
        live.subscribers.forEach(function (subscriber) {
          document.querySelectorAll('[data-subscriber="' + subscriber.id + '"]').forEach(function (cell) {
            cell.textContent = text(subscriber[cell.getAttribute('data-field')]);
          });
        });
        // A subscriber came or went: the table and the diagram are drawn
        // by the server. A person who types a reason is not interrupted.
        var active = document.activeElement;
        var typing = active && active.tagName === 'INPUT';
        if (String(live.overview.subscribers) !== shown && !typing) { location.reload(); }
      })
      .catch(function () {});
  }
  setInterval(tick, 2000);
})();
