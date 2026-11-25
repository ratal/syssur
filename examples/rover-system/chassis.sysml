package ChassisPkg {
    import Power::*;

    port def DriveTorquePort {
        out item torque : Real;
    }

    part def Wheel {
        attribute diameterMm : Real = 320.0;
        port driveIn : ~DriveTorquePort;
    }

    part def SteerableWheel :> Wheel {
        attribute maxSteerDeg : Real = 35.0;
    }

    part def Chassis {
        part wheel : Wheel[4];
        part frontLeftWheel :> wheel : SteerableWheel;
        part frontRightWheel :> wheel : SteerableWheel;
        part rearLeftWheel :> wheel;
        part rearRightWheel :> wheel;
    }
}
