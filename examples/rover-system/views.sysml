package RoverViews {
    import RoverSystem::*;
    import Power::*;
    import ChassisPkg::*;

    view def StructuralBreakdown {
        expose RoverSystem::**;
        expose ChassisPkg::**;
    }

    view def PowerInterfaceView {
        expose Power::**;
    }
}
