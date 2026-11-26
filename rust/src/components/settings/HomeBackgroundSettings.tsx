import React, { useState } from 'react';
import { Check, ImagePlus, Minus, Plus } from 'lucide-react';
import { DEFAULT_HOME_BACKGROUND, homeBackgroundCss, type GradientStop, type HomeBackground, type HomeBackgroundMode } from '../../shared/settings';
import { SPECIAL_ART } from '../Homescreen';
import { Segmented } from '../home/WidgetEditor';

const SOLIDS = ['#1E2A26', '#1F2633', '#2B2233', '#33261F', '#14161A', '#3A3F47', '#F3EFE6', '#E8EEF2', '#EFE6EC', '#E9F0E6', '#F6E7D8', '#DCE3F0'];

const GRADIENTS: { name: string; type: 'linear' | 'radial'; angle: number; stops: GradientStop[] }[] = [
  { name: 'Dusk', type: 'linear', angle: 160, stops: [{ color: '#283C63', at: 0 }, { color: '#B06A6A', at: 55 }, { color: '#F0B27A', at: 100 }] },
  { name: 'Lagoon', type: 'linear', angle: 135, stops: [{ color: '#0F3443', at: 0 }, { color: '#34E89E', at: 100 }] },
  { name: 'Orchard', type: 'linear', angle: 200, stops: [{ color: '#F6D365', at: 0 }, { color: '#FDA085', at: 100 }] },
  { name: 'Ink', type: 'radial', angle: 0, stops: [{ color: '#2A2F3A', at: 0 }, { color: '#0B0C10', at: 100 }] },
  { name: 'Mist', type: 'linear', angle: 180, stops: [{ color: '#E6E9F0', at: 0 }, { color: '#EEF1F5', at: 100 }] },
  { name: 'Aurora', type: 'linear', angle: 120, stops: [{ color: '#1A2A6C', at: 0 }, { color: '#B21F1F', at: 50 }, { color: '#FDBB2D', at: 100 }] },
];

const HEX = /^#[0-9a-f]{6}$/i;

const MODES: { id: HomeBackgroundMode; label: string }[] = [
  { id: 'theme', label: 'Space colours' },
  { id: 'solid', label: 'Colour' },
  { id: 'gradient', label: 'Gradient' },
  { id: 'image', label: 'Your picture' },
  { id: 'special', label: 'Serica Special' },
];

/** A colour field: the system picker plus an exact hex value. */
const ColorField: React.FC<{ value: string; onChange: (hex: string) => void; label: string }> = ({ value, onChange, label }) => {
  const [text, setText] = useState(value);
  React.useEffect(() => setText(value), [value]);
  return (
    <span className="colorfield">
      <input type="color" value={value} onChange={(e) => onChange(e.target.value.toUpperCase())} aria-label={`${label} colour`} />
      <input
        className="input hex"
        value={text}
        maxLength={7}
        spellCheck={false}
        aria-label={`${label} hex value`}
        onChange={(e) => {
          const v = e.target.value.startsWith('#') ? e.target.value : `#${e.target.value}`;
          setText(v);
          if (HEX.test(v)) onChange(v.toUpperCase());
        }}
        onBlur={() => setText(value)}
        onKeyDown={(e) => e.stopPropagation()}
      />
    </span>
  );
};

/** Settings › Home › Background: what fills the new-tab page. */
export const HomeBackgroundSettings: React.FC<{ value: HomeBackground; onChange: (bg: HomeBackground) => void }> = ({ value, onChange }) => {
  const bg = value ?? DEFAULT_HOME_BACKGROUND;
  const [error, setError] = useState<string | null>(null);
  const set = (patch: Partial<HomeBackground>) => onChange({ ...bg, ...patch });
  const setGradient = (patch: Partial<HomeBackground['gradient']>) => set({ gradient: { ...bg.gradient, ...patch } });

  const pick = async () => {
    setError(null);
    try {
      const r = await window.serica?.pickBackgroundImage?.();
      if (r) set({ mode: 'image', image: { ...bg.image, file: r.file } });
    } catch (e) {
      setError(e === 'too-large' ? 'That picture is too large (40 MB at most).' : e === 'not-an-image' ? 'That file isn’t a picture Serica can show.' : 'The picture couldn’t be used.');
    }
  };

  const preview: React.CSSProperties =
    bg.mode === 'solid' || bg.mode === 'gradient'
      ? { background: homeBackgroundCss(bg)! }
      : bg.mode === 'image' && bg.image.file
        ? { backgroundImage: `url("http://sericabg.localhost/${encodeURIComponent(bg.image.file)}")`, backgroundSize: bg.image.fit === 'contain' ? 'contain' : bg.image.fit === 'center' ? 'auto' : 'cover', backgroundPosition: 'center', backgroundRepeat: 'no-repeat', backgroundColor: '#111' }
        : bg.mode === 'special'
          ? { backgroundImage: `url(${SPECIAL_ART[0].thumb})`, backgroundSize: 'cover', backgroundPosition: 'center' }
          : { background: 'var(--theme-bg-canvas)' };

  return (
    <div className="hbg" data-setting-id="home.background">
      <div className="hbg-top">
        <div className="hbg-preview" style={preview} aria-hidden="true">
          {bg.mode === 'image' && bg.image.dim > 0 && <span className="hbg-dim" style={{ opacity: bg.image.dim }} />}
          <span className="hbg-clock tnum">10:24</span>
          <span className="hbg-w a" />
          <span className="hbg-w b" />
        </div>
        <div className="hbg-modes" role="radiogroup" aria-label="Background">
          {MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              role="radio"
              aria-checked={bg.mode === m.id}
              className={`hbg-mode${bg.mode === m.id ? ' on' : ''}`}
              onClick={() => {
                if (m.id === 'image' && !bg.image.file) void pick();
                else set({ mode: m.id });
              }}
            >
              <span className={`hbg-mode-swatch m-${m.id}`} style={m.id === 'special' ? { backgroundImage: `url(${SPECIAL_ART[1].thumb})` } : m.id === 'gradient' ? { background: homeBackgroundCss({ ...bg, mode: 'gradient' })! } : m.id === 'solid' ? { background: bg.solid } : undefined} />
              <span>{m.label}</span>
              {bg.mode === m.id && <Check size={12} className="hbg-mode-check" aria-hidden="true" />}
            </button>
          ))}
        </div>
      </div>

      {bg.mode === 'theme' && <p className="s2-note">The new tab follows the space’s colours from Theme Studio.</p>}

      {bg.mode === 'solid' && (
        <div className="hbg-editor">
          <div className="hbg-swatches" role="radiogroup" aria-label="Colours">
            {SOLIDS.map((c) => (
              <button key={c} type="button" role="radio" aria-checked={bg.solid === c} className={`hbg-swatch${bg.solid === c ? ' on' : ''}`} style={{ background: c }} title={c} onClick={() => set({ solid: c })} />
            ))}
          </div>
          <div className="hbg-line">
            <span>Exact colour</span>
            <ColorField value={bg.solid} label="Background" onChange={(solid) => set({ solid })} />
          </div>
        </div>
      )}

      {bg.mode === 'gradient' && (
        <div className="hbg-editor">
          <div className="hbg-gradients" role="radiogroup" aria-label="Gradients">
            {GRADIENTS.map((g) => (
              <button
                key={g.name}
                type="button"
                className="hbg-gradient"
                style={{ background: homeBackgroundCss({ ...bg, mode: 'gradient', gradient: { type: g.type, angle: g.angle, stops: g.stops } })! }}
                title={g.name}
                aria-label={g.name}
                onClick={() => setGradient({ type: g.type, angle: g.angle, stops: g.stops.map((s) => ({ ...s })) })}
              />
            ))}
          </div>
          <div className="hbg-line">
            <span>Shape</span>
            <Segmented label="Shape" value={bg.gradient.type} options={[{ value: 'linear', label: 'Linear' }, { value: 'radial', label: 'Radial' }]} onChange={(type) => setGradient({ type })} />
          </div>
          {bg.gradient.type === 'linear' && (
            <div className="hbg-line">
              <span>Direction</span>
              <span className="hbg-angle">
                <Segmented
                  label="Direction"
                  value={[0, 45, 90, 135, 180, 225, 270, 315].includes(bg.gradient.angle) ? bg.gradient.angle : -1}
                  options={[0, 45, 90, 135, 180, 225, 270, 315].map((a) => ({ value: a, label: ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'][a / 45] }))}
                  onChange={(angle) => setGradient({ angle })}
                />
                <input className="input num" type="number" min={0} max={359} value={bg.gradient.angle} onChange={(e) => setGradient({ angle: ((Math.round(Number(e.target.value)) % 360) + 360) % 360 })} onKeyDown={(e) => e.stopPropagation()} aria-label="Angle in degrees" />
                <span className="wed-hint">°</span>
              </span>
            </div>
          )}
          <div className="hbg-stops">
            {bg.gradient.stops.map((s, i) => (
              <div key={i} className="hbg-stop">
                <ColorField label={`Stop ${i + 1}`} value={s.color} onChange={(color) => setGradient({ stops: bg.gradient.stops.map((x, k) => (k === i ? { ...x, color } : x)) })} />
                <input
                  className="input num"
                  type="number"
                  min={0}
                  max={100}
                  value={s.at}
                  onChange={(e) => setGradient({ stops: bg.gradient.stops.map((x, k) => (k === i ? { ...x, at: Math.max(0, Math.min(100, Number(e.target.value) || 0)) } : x)) })}
                  onKeyDown={(e) => e.stopPropagation()}
                  aria-label={`Stop ${i + 1} position`}
                />
                <span className="wed-hint">%</span>
                <button type="button" className="btn btn-quiet btn-icon" disabled={bg.gradient.stops.length <= 2} aria-label={`Remove stop ${i + 1}`} onClick={() => setGradient({ stops: bg.gradient.stops.filter((_, k) => k !== i) })}>
                  <Minus size={13} />
                </button>
              </div>
            ))}
            <button
              type="button"
              className="btn btn-quiet"
              disabled={bg.gradient.stops.length >= 5}
              onClick={() => {
                const last = bg.gradient.stops[bg.gradient.stops.length - 1];
                setGradient({ stops: [...bg.gradient.stops.map((x) => ({ ...x, at: Math.round(x.at * 0.8) })), { color: last.color, at: 100 }] });
              }}
            >
              <Plus size={13} /> Add a colour
            </button>
          </div>
        </div>
      )}

      {bg.mode === 'image' && (
        <div className="hbg-editor">
          <div className="hbg-line">
            <span>Picture</span>
            <button type="button" className="btn" onClick={pick}>
              <ImagePlus size={13} aria-hidden="true" /> Choose another…
            </button>
          </div>
          <div className="hbg-line">
            <span>Fit</span>
            <Segmented label="Fit" value={bg.image.fit} options={[{ value: 'cover', label: 'Fill' }, { value: 'contain', label: 'Fit' }, { value: 'center', label: 'Centre' }]} onChange={(fit) => set({ image: { ...bg.image, fit } })} />
          </div>
          <div className="hbg-line">
            <span>Dim</span>
            <Segmented
              label="Dim"
              value={bg.image.dim < 0.08 ? 0 : bg.image.dim < 0.25 ? 0.15 : bg.image.dim < 0.42 ? 0.35 : 0.5}
              options={[{ value: 0, label: 'None' }, { value: 0.15, label: 'A little' }, { value: 0.35, label: 'More' }, { value: 0.5, label: 'Most' }]}
              onChange={(dim) => set({ image: { ...bg.image, dim } })}
            />
          </div>
          <p className="s2-note">The picture is copied into Serica, so moving or deleting the original doesn’t change the background.</p>
        </div>
      )}

      {bg.mode === 'special' && (
        <div className="hbg-editor">
          <div className="hbg-art">
            {SPECIAL_ART.map((a) => (
              <figure key={a.src}>
                <img src={a.thumb} alt="" loading="lazy" />
                <figcaption>
                  <span>{a.title}</span>
                  <span className="wed-hint">{a.after}</span>
                </figcaption>
              </figure>
            ))}
          </div>
          <p className="s2-note">Three scenes of the French Revolution, 1789–1794, re-painted in oil pastel. They cross-fade from one to the next.</p>
          <div className="hbg-line">
            <span>Change every</span>
            <Segmented label="Change every" value={bg.special.seconds} options={[{ value: 30, label: '30 s' }, { value: 45, label: '45 s' }, { value: 120, label: '2 min' }, { value: 600, label: '10 min' }]} onChange={(seconds) => set({ special: { seconds } })} />
          </div>
        </div>
      )}
      {error && <p className="wed-error">{error}</p>}
    </div>
  );
};

export default HomeBackgroundSettings;
