// Shared static-site interactions; the content itself is rendered in each language.
// FAQ links open their answer. Native links/details also work without JavaScript.
function revealLinkedAnswer() {
  const target = document.getElementById(location.hash.slice(1));
  if (target instanceof HTMLDetailsElement) target.open = true;
}
window.addEventListener('hashchange', revealLinkedAnswer);
document.querySelectorAll('a[href^="#"]').forEach(link => {
  link.addEventListener('click', () => {
    const target = document.getElementById(link.getAttribute('href').slice(1));
    if (target instanceof HTMLDetailsElement) target.open = true;
  });
});
document.querySelectorAll('[data-language]').forEach(link => {
  link.addEventListener('click', () => {
    const target = new URL(link.href);
    target.hash = location.hash;
    link.href = target.href;
  });
});
revealLinkedAnswer();
