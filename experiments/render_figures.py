#!/usr/bin/env python3
"""Deterministic article figures from verified published results. Requires matplotlib."""
import json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from reproduce import ROOT, RESULTS, report


def render():
    result = report()
    out = ROOT.parent / 'news/assets'
    out.mkdir(parents=True, exist_ok=True)
    rows = json.loads((RESULTS/'deliveries.json').read_text())['rows']
    plt.rcParams.update({'svg.hashsalt':'hiroute-news-20261004','svg.fonttype':'none','font.family':'DejaVu Sans'})
    from decimal import Decimal
    accepted = [p['pair'] for p in result['pairs'] if p['accepted']]
    costs = {}
    for group in ['strong', 'mixed']:
        selected = [next(r for r in rows if r['id'] == f"formal-draft98-v1-p{number:02d}-{group}") for number in accepted]
        costs[group] = tuple(sum(Decimal(row['usd'][bound]) for row in selected) / len(selected) for bound in ['lower', 'upper'])
    ratio = costs['mixed'][1] / costs['strong'][0]
    for lang in ['zh', 'en']:
        # SVG text remains selectable. Browser font fallback renders the Chinese
        # labels without requiring a platform-specific font in the generator.
        fig, ax = plt.subplots(figsize=(10, 4.8))
        zh = lang == 'zh'
        labels = ['全 Astra', 'HiRoute 混合'] if zh else ['All Astra', 'HiRoute mixed']
        for n, group in enumerate(['strong', 'mixed']):
            lo, hi = costs[group]
            percent = 100 if group == 'strong' else float(ratio * 100)
            color = '#5564bd' if group == 'strong' else '#078677'
            ax.barh(n, percent, height=.46, color=color)
            ax.text(percent + 2, n, f"${lo:.3f}–{hi:.3f}", va='center', fontsize=12, fontweight='bold', color='#192439')
        ax.set_yticks([0, 1], labels); ax.invert_yaxis(); ax.set_xlim(0, 132)
        ax.set_xticks([0, 25, 50, 75, 100], ['0%', '25%', '50%', '75%', '100%'])
        ax.set_xlabel('整单 API 等价成本 · 全 Astra = 100%' if zh else 'Whole-task API-equivalent cost · All Astra = 100%')
        ax.spines[['top', 'right', 'left']].set_visible(False)
        ax.set_title('相同交付标准，成本降低超过 90%' if zh else 'Same delivery standard. Over 90% lower cost.', loc='left', pad=20, fontsize=19)
        ax.text(130, 1, f"−{(1-ratio)*100:.2f}%", va='center', ha='right', fontsize=23, fontweight='bold', color='#087464')
        note = ('两次双方均达标运行的平均成本，包含全部尝试与路由评估。\n保守比较：混合成本上界 ÷ Astra 成本下界；完整三次结果见实验记录。' if zh else 'Mean cost across two runs meeting the same gate, including all attempts and routing evaluation.\nConservative comparison: mixed upper bound ÷ Astra lower bound. All three runs remain in the record.')
        fig.text(.08, .035, note, fontsize=9, color='#566078')
        fig.tight_layout(rect=[0, .14, 1, 1])
        target = out / f'research-cost-{lang}.svg'
        fig.savefig(target, metadata={'Date': None})
        target.write_text('\n'.join(line.rstrip() for line in target.read_text().splitlines()) + '\n')
        plt.close(fig)
    for lang in ['zh','en']:
        labels=(['长任务自动模型接力','Qwen 执行','上下文交接 · 重新评估','Astra 接续','25 次模型请求','0.485 < 0.5 胜任度下限','13 次实际工具调用','31 分 36 秒 · 零中途人工提示 · 343 项独立验收通过','实际接力记录：从研究探查转入实现、修复与验收。'] if lang=='zh' else ['Automatic model handoff','Qwen executes','Context handoff · Reassess','Astra continues','25 model requests','Competence 0.485 < 0.5 floor','13 actual tool calls','31m 36s · No intermediate operator prompts · 343 passing assertions','Recorded HTTPX handoff: from investigation to implementation, repair and acceptance.'])
        from html import escape
        svg=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 320" role="img"><title>'+escape(labels[0])+'</title><rect width="1100" height="320" rx="20" fill="#eef2fa"/><g font-family="system-ui,sans-serif" fill="#192439">',f'<text x="40" y="48" font-size="26" font-weight="700">{escape(labels[0])}</text>']
        for i,x in enumerate([40,410,780]):
            svg.append(f'<rect x="{x}" y="85" width="280" height="110" rx="14" fill="white"/><text x="{x+20}" y="123" font-size="20" font-weight="700">{escape(labels[i+1])}</text><text x="{x+20}" y="163" font-size="16">{escape(labels[i+4])}</text>')
            if i<2:svg.append(f'<path d="M {x+290} 140 h 65 m -10 -7 l 10 7 -10 7" fill="none" stroke="#586caa" stroke-width="3"/>')
        svg.extend([f'<text x="40" y="246" font-size="22" font-weight="700">{escape(labels[7])}</text>',f'<text x="40" y="287" font-size="15" fill="#536079">{escape(labels[8])}</text>','</g></svg>'])
        (out/f'unattended-handoff-{lang}.svg').write_text(''.join(svg)+'\n')


if __name__=='__main__':render()
