// Tráiler de 15 s de la página: una línea de tiempo que va poniendo
// clases en el escenario (las transiciones las hace trailer.css) y
// escribe los textos letra por letra. Da la vuelta sola; se pausa fuera
// de la vista, con la pestaña oculta o con el botón, y con "reducir
// movimiento" queda quieto en un cuadro.
(function () {
  var box = document.querySelector('.trailer');
  var stage = box && box.querySelector('.stage');
  if (!stage) return;

  var LOOP = 15;
  // [segundo, clase]
  var cues = [
    [0.05, 'c-open'], [0.35, 'c-n1'], [0.6, 'cap1'], [2.95, 'c-emo'],
    [3.1, 'cap2'], [3.2, 'c-cur1'], [3.75, 'c-click1'], [3.8, 'c-check1'],
    [4.15, 'c-cur2'], [4.45, 'c-sel'], [4.85, 'c-bold'], [5.2, 'c-fmtoff'], [5.25, 'c-bullet'],
    [6.0, 'cap3'], [6.0, 'c-glass'], [6.5, 'c-n2'], [6.6, 'c-cur3'],
    [8.1, 'cap4'], [8.2, 'c-roll'], [8.9, 'c-cur4'], [9.45, 'c-open1'],
    [10.4, 'cap5'], [10.4, 'c-app'], [10.85, 'c-keys'], [11.15, 'c-picker'], [11.75, 'c-filter'], [12.15, 'c-pick'],
    [12.9, 'c-outro'], [13.5, 'c-chips'], [14.5, 'c-end']
  ];

  // Textos que se escriben: data-type="inicio duración".
  var typed = Array.prototype.map.call(stage.querySelectorAll('[data-type]'), function (el) {
    var p = el.getAttribute('data-type').split(' ');
    var chars = Array.from(el.textContent);
    el.textContent = '';
    return { el: el, chars: chars, start: +p[0], dur: +p[1], n: 0 };
  });

  var last = -1;
  function apply(t) {
    if (t < last) {
      // Vuelta nueva (con la pantalla en negro): todo a cero, sin animar.
      stage.classList.add('reset');
      cues.forEach(function (c) { stage.classList.remove(c[1]); });
      void stage.offsetWidth;
      stage.classList.remove('reset');
    }
    last = t;
    cues.forEach(function (c) {
      if ((t >= c[0]) !== stage.classList.contains(c[1])) stage.classList.toggle(c[1]);
    });
    typed.forEach(function (x) {
      var k = Math.max(0, Math.min(1, (t - x.start) / x.dur));
      var n = Math.round(k * x.chars.length);
      if (n !== x.n) {
        x.el.textContent = x.chars.slice(0, n).join('');
        x.n = n;
      }
    });
  }

  // "?t=segundos" en la dirección: ese cuadro, quieto (para revisarlo).
  var fixed = /[?&]t=([\d.]+)/.exec(location.search);
  var reduce = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (fixed || reduce) {
    stage.classList.add('still');
    apply(fixed ? +fixed[1] : 7.4);
    var btn = box.querySelector('.toggle');
    if (btn) btn.hidden = true;
    return;
  }

  var elapsed = 0, prev = null, paused = false, visible = true;
  function frame(now) {
    if (prev !== null && !paused && visible && !document.hidden) {
      elapsed = (elapsed + (now - prev) / 1000) % LOOP;
      apply(elapsed);
    }
    prev = now;
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  if ('IntersectionObserver' in window) {
    new IntersectionObserver(function (e) { visible = e[0].isIntersecting; }, { threshold: 0.2 }).observe(box);
  }

  var toggle = box.querySelector('.toggle');
  if (toggle) {
    toggle.addEventListener('click', function () {
      paused = !paused;
      box.classList.toggle('paused', paused);
      toggle.setAttribute('aria-label', paused ? 'Reproducir el tráiler' : 'Pausar el tráiler');
    });
  }
})();
