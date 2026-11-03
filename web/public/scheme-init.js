// Runs before the first paint, from index.html. The map is the styles that are dark or light by nature.
try {
  var s = localStorage.getItem('vesper.scheme'),
    f = { persona: 'dark', cozy: 'light', bbs: 'dark', aurora: 'dark' }[localStorage.getItem('vesper.style')],
    l = f ? f === 'light' : s === 'light' || (s === 'system' && matchMedia('(prefers-color-scheme: light)').matches),
    d = document.documentElement;
  d.setAttribute('data-scheme', l ? 'light' : 'dark');
  document.querySelector('meta[name=theme-color]').setAttribute('content', l ? '#fbfaf8' : '#100e0c');
} catch (e) {}
