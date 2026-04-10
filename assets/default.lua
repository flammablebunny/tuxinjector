-- Tuxinjector - Default Configuration
-- Place this file at ~/.config/tuxinjector/init.lua
-- Changes are hot-reloaded automatically.

local ti = require("tuxinjector")

-- ============================================================================
-- Mirrors
-- ============================================================================
-- Captures a region of the game framebuffer and renders it on the overlay.
-- Input positions use anchors (pieLeft, pieRight, topLeftViewport, etc.)
-- so they auto-adapt to any resolution (1080p, 1440p, 4K, ...).

local mirrors = {
    {
        -- Pie chart from F3+S, displayed as circle. Passthrough keeps original RGB.
        name = "pieChart",
        captureWidth = 319,
        captureHeight = 169,
        colorPassthrough = true,
        colorSensitivity = 0.001,
        colors = {
            targetColors = {
                {70, 206, 102},    -- green (unspecified)
                {236, 110, 78},    -- orange (blockentities)
                {228, 70, 196},    -- pink
                {204, 108, 70},    -- brown
                {70, 76, 70},      -- dark green
            },
        },
        border = {
            type = "Static",
            staticShape = "Circle",
            staticColor = {98, 5, 113},
            staticThickness = 5,
            staticRadius = 172,
            staticWidth = 238,
            staticHeight = 235,
            staticOffsetX = 1,
            staticOffsetY = -7,
            dynamicThickness = 4,
        },
        input = {
            { relativeTo = "pieLeft", x = -238, y = -180 },
        },
        output = {
            relativeTo = "centerViewport",
            x = 410,
            y = 31,
            scale = 1.14,
            separateScale = true,
            scaleX = 0.76,
            scaleY = 1.5,
        },
    },
    {
        -- Entity counter from F3 overlay. 23x7 capture x8 = 184x56 output.
        name = "eCounter",
        captureWidth = 23,
        captureHeight = 7,
        nearestFilter = true,
        colors = {
            targetColors = { {221, 221, 221} },   -- F3 text
            output = {255, 255, 255},
            border = {0, 0, 0},
        },
        colorSensitivity = 0.001,
        border = { type = "Dynamic", dynamicThickness = 2 },
        input = {
            { relativeTo = "topLeftViewport", x = 14, y = 38 },
        },
        output = {
            relativeTo = "centerViewport",
            x = 362,
            y = 169,
            scale = 8.0,
        },
    },
    {
        -- Blockentities number (left of pie text). 11x7 capture x8 = 88x56.
        name = "blockentitiesLeft",
        captureWidth = 11,
        captureHeight = 7,
        nearestFilter = true,
        colors = {
            targetColors = { {233, 109, 77} },    -- orange pie text
            output = {243, 169, 78},               -- amber replacement
            border = {90, 54, 14},
        },
        colorSensitivity = 0.001,
        border = { type = "Dynamic", dynamicThickness = 3 },
        input = {
            { relativeTo = "pieLeft", x = 0, y = 0 },
            { relativeTo = "pieLeft", x = 0, y = 8 },
            { relativeTo = "pieLeft", x = 0, y = 16 },
            { relativeTo = "pieLeft", x = 0, y = 24 },
        },
        output = {
            relativeTo = "centerViewport",
            x = 416,
            y = -38,
            scale = 8.0,
        },
    },
    {
        -- Green "unspecified" number (left of pie text). 11x7 capture x8 = 88x56.
        name = "unspecifiedLeft",
        captureWidth = 11,
        captureHeight = 7,
        nearestFilter = true,
        colorPassthrough = true,
        colors = {
            targetColors = {
                {69, 204, 101},
                {69, 203, 101},
            },
            border = {51, 88, 48},
        },
        colorSensitivity = 0.001,
        border = { type = "Dynamic", dynamicThickness = 3 },
        input = {
            { relativeTo = "pieLeft", x = 0, y = 0 },
            { relativeTo = "pieLeft", x = 0, y = 8 },
            { relativeTo = "pieLeft", x = 0, y = 16 },
            { relativeTo = "pieLeft", x = 0, y = 24 },
        },
        output = {
            relativeTo = "centerViewport",
            x = 416,
            y = 23,
            scale = 8.0,
        },
    },
    {
        -- Mapless number (right of pie text). 19x7 capture x8 = 152x56.
        name = "mapless",
        captureWidth = 19,
        captureHeight = 7,
        nearestFilter = true,
        colors = {
            targetColors = { {233, 109, 77} },    -- orange pie text
            output = {255, 255, 255},
            border = {0, 0, 0},
        },
        colorSensitivity = 0.001,
        border = { type = "Dynamic", dynamicThickness = 2 },
        input = {
            { relativeTo = "pieRight", x = 0, y = 0 },
            { relativeTo = "pieRight", x = 0, y = 8 },
            { relativeTo = "pieRight", x = 0, y = 16 },
            { relativeTo = "pieRight", x = 0, y = 24 },
        },
        output = {
            relativeTo = "bottomRightViewport",
            x = 71,
            y = 242,
            scale = 8.0,
        },
    },
    {
        -- Eye measurement mirror. Center strip of 384x16384 tall viewport.
        -- eyezoomLink = true makes position/size match the eyezoom layout automatically.
        name = "eyeMirror",
        captureWidth = 60,
        captureHeight = 580,
        nearestFilter = true,
        rawOutput = true,
        colors = { enabled = false },
        input = {
            { x = 162, y = 7902 },
        },
        output = {
            eyezoomLink = true,
        },
    },
}

-- ============================================================================
-- Images
-- ============================================================================
-- Image overlays rendered on top of the game. Paths do support `~/` expansion.

local images = {
    {
        -- Auto-generated eyezoom overlay (grid + crosshair + labels).
        -- eyezoomLink = true makes position/size match the eyezoom layout automatically.
        name = "measuringOverlay",
        path = "~/.local/share/tuxinjector/images/overlay.png",
        eyezoomLink = true,
    },
}

-- ============================================================================
-- Mirror Groups
-- ============================================================================
-- Groups position mirrors relative to a shared anchor. The blockentities and
-- unspecified text overlays are rendered ON TOP of the pie chart circle.

local mirrorGroups = {
    {
        name = "Preemptive Pie",
        output = {
            relativeTo = "centerViewport",
            x = 0,
            y = 0,
            scale = 1.0,
        },
        mirrors = {
            { mirrorId = "pieChart",          enabled = true, offsetX = 360, offsetY = 0 },
            { mirrorId = "blockentitiesLeft", enabled = true, offsetX = 416, offsetY = -38 },
            { mirrorId = "unspecifiedLeft",   enabled = true, offsetX = 416, offsetY = 23 },
        },
    },
}

-- ============================================================================
-- Gradient background shared by non-Fullscreen modes.
-- ============================================================================

local mode_gradient_bg = {
    selectedMode = "gradient",
    gradientAngle = 45.0,
    gradientAnimation = "wave",
    gradientAnimationSpeed = 0.5,
    gradientStops = {
        { color = {84, 11, 128, 255}, position = 0.0 },
        { color = {21, 0, 72, 255}, position = 1.0 },
    },
}

-- Border shared by non-Fullscreen modes.
local mode_border = {
    enabled = true,
    color = {122, 21, 162, 255},
    width = 1,
}

-- ============================================================================
-- Modes
-- ============================================================================

local modes = {
    {
        id = "Fullscreen",
        useRelativeSize = true,
        relativeWidth = 1.0,
        relativeHeight = 1.0,
        gameTransition = "Bounce",
        transitionDurationMs = 300,
        easeOutPower = 3.0,
        slideMirrorsIn = true,
        mirrorIds = { "mapless" },
        border = {
            enabled = false,
        },
        background = {
            selectedMode = "color",
            color = {0, 0, 0, 0},
        },
    },
    {
        -- Thin: max(330, sw/8) wide, 95% screen height.
        id = "Thin",
        widthExpr = "max(330, roundEven(sw / 8))",
        heightExpr = "roundEven(sh * 0.95)",
        gameTransition = "Bounce",
        transitionDurationMs = 300,
        easeOutPower = 3.0,
        bounceIntensity = 0.02,
        bounceDurationMs = 200,
        slideMirrorsIn = true,
        mirrorGroupIds = { "Preemptive Pie" },
        mirrorIds = { "mapless", "eCounter" },
        border = mode_border,
        background = mode_gradient_bg,
    },
    {
        -- Wide: 98% screen width, 25% height.
        id = "Wide",
        widthExpr = "roundEven(sw * 0.98)",
        useRelativeSize = true,
        relativeHeight = 0.25,
        gameTransition = "Bounce",
        transitionDurationMs = 300,
        easeOutPower = 3.0,
        slideMirrorsIn = true,
        border = mode_border,
        background = mode_gradient_bg,
    },
    {
        -- Tall / EyeZoom: extreme height stretches game vertically.
        id = "Tall",
        width = 384,
        height = 16384,
        enableEyezoom = true,
        gameTransition = "Bounce",
        transitionDurationMs = 300,
        easeOutPower = 3.0,
        skipAnimateY = true,
        slideMirrorsIn = true,
        mirrorGroupIds = { "Preemptive Pie" },
        mirrorIds = { "eyeMirror", "eCounter" },
        imageIds = { "measuringOverlay" },
        border = { enabled = false },
        background = mode_gradient_bg,
    },
    {
        -- Preemptive: Tall without images, for pie viewing during stronghold nav.
        id = "Preemptive",
        width = 384,
        height = 16384,
        gameTransition = "Bounce",
        transitionDurationMs = 300,
        easeOutPower = 3.0,
        skipAnimateY = true,
        slideMirrorsIn = true,
        mirrorGroupIds = { "Preemptive Pie" },
        mirrorIds = { "eCounter" },
        border = { enabled = false },
        background = mode_gradient_bg,
    },
}

-- ============================================================================
-- Hotkeys
-- ============================================================================
-- Key names (or GLFW keycodes). Debounce 100ms, F3 excluded.

local hotkeys = {
    {
        keys = {"Z"},
        mainMode = "Fullscreen",
        secondaryMode = "Thin",
        debounce = 100,
        blockKeyFromGame = true,
        conditions = { exclusions = {"F3"} },
    },
    {
        keys = {"J"},
        mainMode = "Fullscreen",
        secondaryMode = "Tall",
        debounce = 100,
        blockKeyFromGame = true,
        conditions = { exclusions = {"F3"} },
    },
    {
        keys = {"Alt"},
        mainMode = "Fullscreen",
        secondaryMode = "Wide",
        debounce = 100,
        blockKeyFromGame = true,
        conditions = { exclusions = {"F3"} },
    },
}

-- ============================================================================
-- EyeZoom config
-- ============================================================================

local eyezoom = {
    cloneWidth = 30,
    cloneHeight = 1300,
    overlayWidth = 30,
    stretchWidth = 810,
    windowWidth = 384,
    windowHeight = 16384,
    horizontalMargin = 0,
    verticalMargin = 0,
    autoFontSize = true,
    textFontSize = 42,
    rectHeight = 40,
    linkRectToFont = false,
    numberStyle = "slackow",
    slideZoomIn = true,
    slideMirrorsIn = true,
    gridColor1 = {255, 182, 193},
    gridColor2 = {173, 216, 230},
    centerLineColor = {255, 255, 255},
    textColor = {0, 0, 0},
}

-- ============================================================================
-- Cursors
-- ============================================================================

local cursors = {
    enabled = false,
    title = { cursorName = "Arrow", cursorSize = 32 },
    wall = { cursorName = "Arrow", cursorSize = 32 },
    ingame = { cursorName = "Cross (Inverted, medium)", cursorSize = 32 },
}

-- ============================================================================
-- Return config
-- ============================================================================

return {
    configVersion = 1,

    display = {
        defaultMode = "Fullscreen",
        fpsLimit = 0,
        disableAnimations = false,
        hideAnimationsInGame = true,
    },

    input = {
        mouseSensitivity = 1.0,
        allowCursorEscape = true,
    },

    theme = {
        cursors = cursors,
    },

    overlays = {
        mirrors = mirrors,
        mirrorGroups = mirrorGroups,
        images = images,
        textOverlays = {},
        eyezoom = eyezoom,
    },

    hotkeys = {
        gui = {"Ctrl", "I"},
        modeHotkeys = hotkeys,
    },

    modes = modes,
}
