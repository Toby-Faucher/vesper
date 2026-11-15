// Runs before the first paint, from index.html: the same rules as src/lib/ui/scheme.ts.
try {
  var s = localStorage.getItem('vesper.scheme'),
    l = s === 'light' || (s === 'system' && matchMedia('(prefers-color-scheme: light)').matches),
    d = document.documentElement;
  d.setAttribute('data-scheme', l ? 'light' : 'dark');
  document.querySelector('meta[name=theme-color]').setAttribute('content', l ? '#fbfaf8' : '#100e0c');
} catch (e) {}
