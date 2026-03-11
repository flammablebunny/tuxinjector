#![allow(clippy::must_use_candidate)]

#[derive(PartialEq, Clone, Copy, Eq)]
pub struct GlVersion {
    pub major: u16,
    pub minor: u16,
    pub is_gles: bool,
}

impl GlVersion {
    pub const fn gl(major: u16, minor: u16) -> Self {
        Self {
            major,
            minor,
            is_gles: false,
        }
    }

    pub const fn gles(major: u16, minor: u16) -> Self {
        Self {
            major,
            minor,
            is_gles: true,
        }
    }

    pub fn read<G: glow::HasContext>(gl: &G) -> Self {
        Self::parse(&unsafe { gl.get_parameter_string(glow::VERSION) })
    }

    pub fn parse(gl_version_string: &str) -> Self {
        let (version_string, is_gles) = gl_version_string
            .strip_prefix("OpenGL ES ")
            .map_or_else(|| (gl_version_string, false), |version| (version, true));

        let mut parts = version_string.split(|c: char| !c.is_numeric());
        let major = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let minor = parts.next().unwrap_or("0").parse().unwrap_or(0);

        Self {
            major,
            minor,
            is_gles,
        }
    }

    #[cfg(feature = "debug_message_insert_support")]
    pub fn debug_message_insert_support(self) -> bool {
        self >= Self::gl(4, 3)
    }

    #[cfg(feature = "bind_vertex_array_support")]
    pub fn bind_vertex_array_support(self) -> bool {
        self.major >= 3
    }

    #[cfg(feature = "vertex_offset_support")]
    pub fn vertex_offset_support(self) -> bool {
        self >= Self::gl(3, 2)
    }

    #[cfg(feature = "bind_sampler_support")]
    pub fn bind_sampler_support(self) -> bool {
        self >= GlVersion::gl(3, 2) || self >= GlVersion::gles(3, 0)
    }

    #[cfg(feature = "clip_origin_support")]
    pub fn clip_origin_support(self) -> bool {
        self >= GlVersion::gl(4, 5)
    }

    #[cfg(feature = "polygon_mode_support")]
    pub fn polygon_mode_support(self) -> bool {
        !self.is_gles
    }

    #[cfg(feature = "primitive_restart_support")]
    pub fn primitive_restart_support(self) -> bool {
        self >= GlVersion::gl(3, 1)
    }
}

impl PartialOrd for GlVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.is_gles == other.is_gles {
            Some(
                self.major
                    .cmp(&other.major)
                    .then(self.minor.cmp(&other.minor)),
            )
        } else {
            None
        }
    }
}

pub struct GlslVersion {
    pub major: u16,
    pub minor: u16,
    pub is_gles: bool,
}

impl GlslVersion {
    pub fn read<G: glow::HasContext>(gl: &G) -> Self {
        Self::parse(&unsafe { gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION) })
    }

    pub fn parse(gl_shading_language_version: &str) -> Self {
        let (version_string, is_gles) = gl_shading_language_version
            .strip_prefix("OpenGL ES GLSL ES ")
            .map_or_else(
                || (gl_shading_language_version, false),
                |version| (version, true),
            );

        let mut parts = version_string.split(|c: char| !c.is_numeric());
        let major = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let minor = parts.next().unwrap_or("0").parse().unwrap_or(0);

        let minor = if minor >= 10 { minor / 10 } else { minor };

        Self {
            major,
            minor,
            is_gles,
        }
    }
}
