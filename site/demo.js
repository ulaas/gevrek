// A small gevrek to try: windows that are opened and closed take their
// places by the layout that is chosen, as the real one places them.
(function () {
  "use strict";
  var display = document.getElementById("display");
  if (!display) return;
  var layoutName = document.getElementById("layout-name");
  var clock = document.getElementById("clock");
  var still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // The programs that are opened, in turn.
  var PROGRAMS = [
    { name: "Terminal", says: "A place to type", tint: "#F4E2BC" },
    { name: "Soyuz", says: "Files", tint: "#E58A3C" },
    { name: "gitem", says: "Git", tint: "#D9701F" },
    { name: "Peek", says: "Another machine", tint: "#9CC4B2" },
    { name: "Corrent", says: "Films and music", tint: "#E0B15C" },
    { name: "walle", says: "Wallpapers", tint: "#C98BA6" }
  ];
  var MOST = PROGRAMS.length;
  var GAP = 0.016;

  // Each layout: where n windows lie, as parts of the display.
  function inset(rect) {
    return [rect[0] + GAP / 2, rect[1] + GAP / 2 * 1.6, rect[2] - GAP, rect[3] - GAP * 1.6];
  }
  var LAYOUTS = [
    {
      name: "Dwindle",
      place: function (n) {
        var rects = [], area = [0, 0, 1, 1];
        for (var i = 0; i < n; i++) {
          if (i === n - 1) { rects.push(area); break; }
          if (i % 2 === 0) {
            rects.push([area[0], area[1], area[2] / 2, area[3]]);
            area = [area[0] + area[2] / 2, area[1], area[2] / 2, area[3]];
          } else {
            rects.push([area[0], area[1], area[2], area[3] / 2]);
            area = [area[0], area[1] + area[3] / 2, area[2], area[3] / 2];
          }
        }
        return rects;
      }
    },
    {
      name: "Main and stack",
      place: function (n) {
        if (n === 1) return [[0, 0, 1, 1]];
        var rects = [[0, 0, 0.58, 1]];
        for (var i = 1; i < n; i++) rects.push([0.58, (i - 1) / (n - 1), 0.42, 1 / (n - 1)]);
        return rects;
      }
    },
    {
      name: "Columns",
      place: function (n) {
        var rects = [];
        for (var i = 0; i < n; i++) rects.push([i / n, 0, 1 / n, 1]);
        return rects;
      }
    },
    {
      name: "Grid",
      place: function (n) {
        var columns = Math.ceil(Math.sqrt(n)), rows = Math.ceil(n / columns), rects = [];
        for (var i = 0; i < n; i++) {
          var row = Math.floor(i / columns);
          var inRow = row === rows - 1 ? n - row * columns : columns;
          rects.push([(i % columns) / inRow, row / rows, 1 / inRow, 1 / rows]);
        }
        return rects;
      }
    },
    {
      name: "Monocle",
      place: function (n) {
        var rects = [];
        for (var i = 0; i < n; i++) rects.push([0, 0, 1, 1]);
        return rects;
      }
    },
    {
      name: "Floating",
      loose: true,
      place: function (n) {
        var rects = [];
        for (var i = 0; i < n; i++) rects.push([0.05 + i * 0.085, 0.07 + ((i * 37) % 5) * 0.065, 0.46, 0.56]);
        return rects;
      }
    }
  ];

  var windows = [];   // { el, program }
  var focused = 0;
  var layout = 5;     // floating first: the page begins as any desktop does
  var opened = 0;

  function make(program) {
    var el = document.createElement("button");
    el.type = "button";
    el.className = "win coming";
    el.style.setProperty("--tint", program.tint);
    var html = '<span class="title">' + program.name + '</span><span class="says">' + program.says + "</span>";
    for (var i = 0; i < 9; i++) html += '<i style="width:' + (38 + ((i * 53 + program.name.length * 17) % 55)) + '%"></i>';
    el.innerHTML = html;
    el.addEventListener("click", function () {
      for (var i = 0; i < windows.length; i++) if (windows[i].el === el) focused = i;
      draw();
    });
    display.appendChild(el);
    return el;
  }

  function draw() {
    var now = LAYOUTS[layout];
    var rects = now.place(windows.length);
    layoutName.textContent = now.name;
    windows.forEach(function (one, index) {
      var rect = now.loose ? rects[index] : inset(rects[index]);
      one.el.style.left = rect[0] * 100 + "%";
      one.el.style.top = rect[1] * 100 + "%";
      one.el.style.width = rect[2] * 100 + "%";
      one.el.style.height = rect[3] * 100 + "%";
      one.el.classList.toggle("focused", index === focused);
      one.el.classList.toggle("hidden", now.name === "Monocle" && index !== focused);
      one.el.style.zIndex = index === focused ? 3 : now.loose ? 1 : "";
      one.el.setAttribute("aria-label", one.program.name + (index === focused ? ", has the keyboard" : ""));
    });
    document.querySelectorAll("#demo [data-do]").forEach(function (button) {
      var what = button.getAttribute("data-do");
      button.disabled = (what === "open" && windows.length >= MOST) || (what === "close" && windows.length <= 1)
        || (what === "swap" && (focused === 0 || windows.length < 2));
    });
  }

  function open() {
    if (windows.length >= MOST) return;
    var taken = windows.map(function (one) { return one.program; });
    var program = PROGRAMS[opened % MOST];
    for (var i = 0; i < MOST && taken.indexOf(program) >= 0; i++) program = PROGRAMS[(opened + i + 1) % MOST];
    opened++;
    var el = make(program);
    windows.push({ el: el, program: program });
    focused = windows.length - 1;
    draw();
    // (Placed first, then shown: it comes in where it will be.)
    requestAnimationFrame(function () { requestAnimationFrame(function () { el.classList.remove("coming"); }); });
  }

  function close() {
    if (windows.length <= 1) return;
    var gone = windows.splice(focused, 1)[0];
    gone.el.remove();
    focused = Math.min(focused, windows.length - 1);
    draw();
  }

  function next() {
    layout = (layout + 1) % LAYOUTS.length;
    draw();
  }

  function first() {
    if (focused === 0 || windows.length < 2) return;
    windows.unshift(windows.splice(focused, 1)[0]);
    focused = 0;
    draw();
  }

  var DO = { open: open, close: close, layout: next, swap: first };
  document.querySelectorAll("#demo [data-do]").forEach(function (button) {
    button.addEventListener("click", function () { DO[button.getAttribute("data-do")](); });
  });
  document.addEventListener("keydown", function (event) {
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    var tag = event.target && event.target.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA") return;
    var key = event.key.toLowerCase();
    var what = { n: open, q: close, l: next, m: first }[key];
    if (what) { what(); event.preventDefault(); }
  });

  function tick() {
    var now = new Date();
    clock.textContent = ("0" + now.getHours()).slice(-2) + ":" + ("0" + now.getMinutes()).slice(-2);
  }
  tick();
  setInterval(tick, 30000);

  // The beginning: four windows lie as on any desktop, then gevrek
  // places them. Once.
  for (var i = 0; i < 4; i++) open();
  focused = 0;
  if (still) {
    layout = 0;
    draw();
  } else {
    draw();
    setTimeout(function () { layout = 0; draw(); }, 1100);
  }
})();
