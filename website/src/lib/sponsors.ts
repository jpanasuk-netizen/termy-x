const neonDarkLogo = '/sponsors/neon-logo-dark-color.svg';
const neonLightLogo = '/sponsors/neon-logo-light-color.svg';

export interface Sponsor {
  name: string;
  /** Shown under the logo; falls back to the name when omitted. */
  description?: string;
  url: string;
  /** Person avatars render as a circle instead of a wordmark. */
  avatar?: boolean;
  logo: {
    light: string;
    dark: string;
  };
}

export const sponsors: Sponsor[] = [
  {
    name: 'Neon',
    description: 'Serverless Postgres for modern applications.',
    url: 'https://neon.tech',
    logo: {
      light: neonLightLogo,
      dark: neonDarkLogo,
    },
  },
  {
    name: 'Dominik Koch',
    url: 'https://github.com/mezotv',
    avatar: true,
    logo: {
      light: 'https://github.com/mezotv.png',
      dark: 'https://github.com/mezotv.png',
    },
  },
];
