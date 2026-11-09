export function architectureLabel(platform, architecture, language) {
  if (platform === 'macos') return architecture === 'arm64'
    ? (language === 'en' ? 'Apple silicon' : 'Apple 芯片') : 'Intel';
  return architecture === 'aarch64' ? 'ARM64' : platform === 'windows' ? 'x64' : architecture;
}

export function installPlatforms(release, language) {
  const t = (zh, en) => language === 'en' ? en : zh;
  return [
    { id: 'macos', name: 'macOS', platform: 'macOS', kind: 'desktop', guide: 'install-macos' },
    { id: 'omarchy', name: 'Omarchy', platform: 'Omarchy', kind: 'desktop', guide: 'install-omarchy' },
    { id: 'windows', name: 'Windows', platform: 'Windows', kind: 'desktop', guide: 'install-windows' },
    { id: 'linux', name: 'Linux Server', platform: 'Linux', kind: 'standalone', guide: 'install-linux' },
  ].map(platform => {
    const artifacts = (release?.artifacts ?? [])
      .filter(artifact => artifact.platform === platform.platform && artifact.kind === platform.kind)
      .sort((a, b) => {
        const first = platform.id === 'macos' ? 'arm64' : 'x86_64';
        return Number(b.architecture === first) - Number(a.architecture === first);
      });
    return {
      ...platform,
      artifacts,
      type: platform.kind === 'desktop' ? t('桌面应用', 'Desktop app') : t('Web UI 与 CLI', 'Web UI and CLI'),
      architectures: artifacts.map(artifact => architectureLabel(platform.id, artifact.architecture, language)).join(' / '),
    };
  });
}
